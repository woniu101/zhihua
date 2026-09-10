use keyring::Entry;
use russh::{
    client,
    keys::{
        decode_secret_key, key::PrivateKeyWithHashAlg, ssh_key::HashAlg, PublicKeyOrCertificate,
    },
    ChannelMsg, Disconnect,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{watch, Mutex},
};

const CREDENTIAL_SERVICE: &str = "cn.zhihua.desktop.ssh-tunnel";
const CREDENTIAL_USER: &str = "private-key";

type TunnelResult<T> = Result<T, TunnelError>;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelError {
    pub code: String,
    pub message: String,
}

impl TunnelError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TunnelConfiguration {
    host: String,
    port: u16,
    username: String,
    host_key_fingerprint: String,
    remote_host: String,
    remote_port: u16,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveTunnelConfigurationInput {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub host_key_fingerprint: String,
    pub private_key: String,
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TunnelPhase {
    #[default]
    Stopped,
    Connecting,
    Connected,
    Reconnecting,
    Error,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    pub configured: bool,
    pub phase: TunnelPhase,
    pub local_url: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Clone)]
struct FingerprintHandler {
    expected: String,
}

impl client::Handler for FingerprintHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let actual = server_public_key
            .public_key()
            .fingerprint(HashAlg::Sha256)
            .to_string();
        Ok(actual == self.expected)
    }
}

pub struct SshTunnelManager {
    metadata_path: PathBuf,
    credential_user: String,
    status: Arc<RwLock<TunnelStatus>>,
    stop_sender: Mutex<Option<watch::Sender<bool>>>,
}

impl SshTunnelManager {
    pub fn new(app_data_dir: PathBuf) -> TunnelResult<Self> {
        fs::create_dir_all(&app_data_dir).map_err(|error| {
            TunnelError::new("TUNNEL_CONFIG_IO", format!("无法创建隧道配置目录：{error}"))
        })?;
        let manager = Self {
            metadata_path: app_data_dir.join("ssh-tunnel.json"),
            credential_user: CREDENTIAL_USER.to_owned(),
            status: Arc::new(RwLock::new(TunnelStatus::default())),
            stop_sender: Mutex::new(None),
        };
        let configured =
            manager.read_configuration()?.is_some() && manager.private_key()?.is_some();
        manager.write_status(|status| status.configured = configured);
        Ok(manager)
    }

    pub fn save_configuration(
        &self,
        input: SaveTunnelConfigurationInput,
    ) -> TunnelResult<TunnelStatus> {
        validate_endpoint(&input.host, input.port)?;
        let username = required_text(input.username, "SSH 用户名")?;
        let fingerprint = normalize_fingerprint(input.host_key_fingerprint)?;
        decode_secret_key(&input.private_key, None).map_err(|_| {
            TunnelError::new("SSH_PRIVATE_KEY_INVALID", "SSH 私钥不是有效的 OpenSSH 私钥")
        })?;
        self.credential_entry()?
            .set_password(&input.private_key)
            .map_err(|error| credential_error("保存", error))?;
        let configuration = TunnelConfiguration {
            host: input.host.trim().to_owned(),
            port: input.port,
            username,
            host_key_fingerprint: fingerprint,
            remote_host: "127.0.0.1".to_owned(),
            remote_port: 8000,
        };
        write_json_atomically(&self.metadata_path, &configuration)?;
        self.write_status(|status| {
            status.configured = true;
            status.last_error = None;
        });
        self.status()
    }

    pub async fn start(&self) -> TunnelResult<TunnelStatus> {
        let mut stop_guard = self.stop_sender.lock().await;
        if stop_guard.is_some() {
            return self.status();
        }
        let configuration = self
            .read_configuration()?
            .ok_or_else(|| TunnelError::new("TUNNEL_NOT_CONFIGURED", "尚未配置 SSH 隧道"))?;
        let private_key = self.private_key()?.ok_or_else(|| {
            TunnelError::new(
                "SSH_PRIVATE_KEY_MISSING",
                "Windows 凭据管理器中没有 SSH 私钥",
            )
        })?;
        self.set_phase(TunnelPhase::Connecting, None, None);
        let session = connect_session(&configuration, &private_key)
            .await
            .map_err(|error| {
                self.set_phase(TunnelPhase::Error, None, Some(error.message.clone()));
                error
            })?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|error| {
                TunnelError::new("LOCAL_BIND_FAILED", format!("无法创建本机入口：{error}"))
            })?;
        let local_address = listener.local_addr().map_err(|error| {
            TunnelError::new("LOCAL_BIND_FAILED", format!("无法读取本机入口：{error}"))
        })?;
        let local_url = format!("http://127.0.0.1:{}", local_address.port());
        self.set_phase(TunnelPhase::Connected, Some(local_url), None);

        let (stop_sender, stop_receiver) = watch::channel(false);
        *stop_guard = Some(stop_sender);
        let status = self.status.clone();
        tauri::async_runtime::spawn(run_supervisor(
            listener,
            configuration,
            private_key,
            session,
            stop_receiver,
            status,
        ));
        self.status()
    }

    pub async fn stop(&self) -> TunnelResult<TunnelStatus> {
        if let Some(sender) = self.stop_sender.lock().await.take() {
            let _ = sender.send(true);
        }
        self.set_phase(TunnelPhase::Stopped, None, None);
        self.status()
    }

    pub async fn read_service_token(&self) -> TunnelResult<String> {
        let configuration = self
            .read_configuration()?
            .ok_or_else(|| TunnelError::new("TUNNEL_NOT_CONFIGURED", "尚未配置 SSH 隧道"))?;
        let private_key = self.private_key()?.ok_or_else(|| {
            TunnelError::new(
                "SSH_PRIVATE_KEY_MISSING",
                "Windows 凭据管理器中没有 SSH 私钥",
            )
        })?;
        let session = connect_session(&configuration, &private_key).await?;
        let mut channel = session.channel_open_session().await.map_err(|error| {
            TunnelError::new(
                "SSH_COMMAND_FAILED",
                format!("无法打开 SSH 命令通道：{error}"),
            )
        })?;
        channel
            .exec(
                true,
                "sed -n 's/^ZHIHUA_SERVICE_TOKEN=//p' /root/zhihua-service/.env",
            )
            .await
            .map_err(|error| {
                TunnelError::new("SSH_COMMAND_FAILED", format!("无法读取服务配置：{error}"))
            })?;
        let mut output = Vec::new();
        let mut exit_status = None;
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Data { data } => {
                    if output.len() + data.len() > 1024 {
                        return Err(TunnelError::new(
                            "SERVICE_TOKEN_INVALID",
                            "远端服务令牌响应过长",
                        ));
                    }
                    output.extend_from_slice(&data);
                }
                ChannelMsg::ExitStatus { exit_status: value } => exit_status = Some(value),
                _ => {}
            }
        }
        let _ = session
            .disconnect(Disconnect::ByApplication, "bootstrap complete", "en")
            .await;
        if exit_status != Some(0) {
            return Err(TunnelError::new(
                "SERVICE_TOKEN_UNAVAILABLE",
                "远端知画服务令牌不可用",
            ));
        }
        let token = String::from_utf8(output)
            .map_err(|_| TunnelError::new("SERVICE_TOKEN_INVALID", "远端服务令牌编码无效"))?;
        let token = token.trim();
        if !(32..=512).contains(&token.chars().count()) {
            return Err(TunnelError::new(
                "SERVICE_TOKEN_INVALID",
                "远端知画服务令牌长度无效",
            ));
        }
        Ok(token.to_owned())
    }

    pub fn status(&self) -> TunnelResult<TunnelStatus> {
        self.status
            .read()
            .map(|status| status.clone())
            .map_err(|_| TunnelError::new("TUNNEL_STATE_ERROR", "SSH 隧道状态锁已损坏"))
    }

    fn read_configuration(&self) -> TunnelResult<Option<TunnelConfiguration>> {
        if !self.metadata_path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&self.metadata_path).map_err(|error| {
            TunnelError::new("TUNNEL_CONFIG_IO", format!("无法读取隧道配置：{error}"))
        })?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| TunnelError::new("TUNNEL_CONFIG_INVALID", "SSH 隧道配置已损坏"))
    }

    fn private_key(&self) -> TunnelResult<Option<String>> {
        match self.credential_entry()?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(credential_error("读取", error)),
        }
    }

    fn credential_entry(&self) -> TunnelResult<Entry> {
        Entry::new(CREDENTIAL_SERVICE, &self.credential_user)
            .map_err(|error| credential_error("打开", error))
    }

    fn set_phase(&self, phase: TunnelPhase, local_url: Option<String>, last_error: Option<String>) {
        self.write_status(|status| {
            status.phase = phase;
            status.local_url = local_url;
            status.last_error = last_error;
        });
    }

    fn write_status(&self, update: impl FnOnce(&mut TunnelStatus)) {
        if let Ok(mut status) = self.status.write() {
            update(&mut status);
        }
    }
}

async fn connect_session(
    configuration: &TunnelConfiguration,
    private_key: &str,
) -> TunnelResult<client::Handle<FingerprintHandler>> {
    let key = decode_secret_key(private_key, None)
        .map_err(|_| TunnelError::new("SSH_PRIVATE_KEY_INVALID", "无法读取保存的 SSH 私钥"))?;
    let config = client::Config {
        nodelay: true,
        keepalive_interval: Some(Duration::from_secs(20)),
        keepalive_max: 3,
        inactivity_timeout: Some(Duration::from_secs(90)),
        ..Default::default()
    };
    let mut session = tokio::time::timeout(
        Duration::from_secs(15),
        client::connect(
            Arc::new(config),
            (configuration.host.as_str(), configuration.port),
            FingerprintHandler {
                expected: configuration.host_key_fingerprint.clone(),
            },
        ),
    )
    .await
    .map_err(|_| TunnelError::new("SSH_CONNECT_TIMEOUT", "连接实例 SSH 超时"))?
    .map_err(|error| TunnelError::new("SSH_CONNECT_FAILED", format!("SSH 连接失败：{error}")))?;
    let authentication = session
        .authenticate_publickey(
            configuration.username.clone(),
            PrivateKeyWithHashAlg::new(Arc::new(key), None),
        )
        .await
        .map_err(|error| TunnelError::new("SSH_AUTH_FAILED", format!("SSH 认证失败：{error}")))?;
    if !authentication.success() {
        return Err(TunnelError::new("SSH_AUTH_FAILED", "SSH 私钥认证未通过"));
    }
    Ok(session)
}

async fn run_supervisor(
    listener: TcpListener,
    configuration: TunnelConfiguration,
    private_key: String,
    initial_session: client::Handle<FingerprintHandler>,
    mut stop: watch::Receiver<bool>,
    status: Arc<RwLock<TunnelStatus>>,
) {
    let mut session = Some(initial_session);
    let mut retry_seconds = 1_u64;
    loop {
        if *stop.borrow() {
            break;
        }
        if session.is_none() {
            write_shared_status(&status, TunnelPhase::Reconnecting, None);
            match connect_session(&configuration, &private_key).await {
                Ok(connected) => {
                    session = Some(connected);
                    retry_seconds = 1;
                    write_shared_status(&status, TunnelPhase::Connected, None);
                }
                Err(error) => {
                    write_shared_status(&status, TunnelPhase::Reconnecting, Some(error.message));
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(retry_seconds)) => {}
                        _ = stop.changed() => {}
                    }
                    retry_seconds = (retry_seconds * 2).min(20);
                    continue;
                }
            }
        }

        tokio::select! {
            _ = stop.changed() => {}
            accepted = listener.accept() => {
                let Ok((socket, origin)) = accepted else {
                    write_shared_status(&status, TunnelPhase::Error, Some("本机隧道入口已关闭".to_owned()));
                    break;
                };
                let result = open_forward(
                    session.as_ref().expect("session checked above"),
                    socket,
                    origin,
                    &configuration,
                ).await;
                match result {
                    Ok((socket, channel)) => {
                        tauri::async_runtime::spawn(copy_forward(socket, channel));
                    }
                    Err(error) => {
                        if let Some(disconnected) = session.take() {
                            let _ = disconnected.disconnect(Disconnect::ByApplication, "reconnect", "en").await;
                        }
                        write_shared_status(&status, TunnelPhase::Reconnecting, Some(error.message));
                    }
                }
            }
        }
    }
    if let Some(connected) = session {
        let _ = connected
            .disconnect(Disconnect::ByApplication, "client stopped", "en")
            .await;
    }
    write_shared_status(&status, TunnelPhase::Stopped, None);
}

async fn open_forward(
    session: &client::Handle<FingerprintHandler>,
    socket: TcpStream,
    origin: SocketAddr,
    configuration: &TunnelConfiguration,
) -> TunnelResult<(TcpStream, russh::Channel<client::Msg>)> {
    let channel = session
        .channel_open_direct_tcpip(
            configuration.remote_host.clone(),
            configuration.remote_port.into(),
            origin.ip().to_string(),
            origin.port().into(),
        )
        .await
        .map_err(|error| {
            TunnelError::new("SSH_FORWARD_FAILED", format!("SSH 转发失败：{error}"))
        })?;
    Ok((socket, channel))
}

async fn copy_forward(mut socket: TcpStream, mut channel: russh::Channel<client::Msg>) {
    let mut socket_closed = false;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        tokio::select! {
            read = socket.read(&mut buffer), if !socket_closed => {
                match read {
                    Ok(0) => {
                        socket_closed = true;
                        let _ = channel.eof().await;
                    }
                    Ok(length) => {
                        if channel.data(&buffer[..length]).await.is_err() { break; }
                    }
                    Err(_) => break,
                }
            }
            message = channel.wait() => {
                match message {
                    Some(ChannelMsg::Data { data }) => {
                        if socket.write_all(&data).await.is_err() { break; }
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                    _ => {}
                }
            }
        }
    }
    let _ = socket.shutdown().await;
}

fn write_shared_status(
    status: &Arc<RwLock<TunnelStatus>>,
    phase: TunnelPhase,
    last_error: Option<String>,
) {
    if let Ok(mut current) = status.write() {
        current.phase = phase;
        current.last_error = last_error;
        if phase == TunnelPhase::Stopped || phase == TunnelPhase::Error {
            current.local_url = None;
        }
    }
}

fn validate_endpoint(host: &str, port: u16) -> TunnelResult<()> {
    let host = host.trim();
    if host.is_empty() || host.len() > 253 || port == 0 {
        return Err(TunnelError::new(
            "SSH_ENDPOINT_INVALID",
            "SSH 地址或端口无效",
        ));
    }
    if host.parse::<IpAddr>().is_err()
        && !host
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'.' | b'-'))
    {
        return Err(TunnelError::new(
            "SSH_ENDPOINT_INVALID",
            "SSH 主机名包含无效字符",
        ));
    }
    Ok(())
}

fn required_text(value: String, label: &str) -> TunnelResult<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 {
        return Err(TunnelError::new(
            "TUNNEL_CONFIG_INVALID",
            format!("{label}无效"),
        ));
    }
    Ok(value.to_owned())
}

fn normalize_fingerprint(value: String) -> TunnelResult<String> {
    let value = value.trim();
    if !value.starts_with("SHA256:") || value.len() < 24 || value.len() > 128 {
        return Err(TunnelError::new(
            "SSH_FINGERPRINT_INVALID",
            "SSH 主机指纹必须使用 SHA256 格式",
        ));
    }
    Ok(value.to_owned())
}

fn credential_error(action: &str, error: keyring::Error) -> TunnelError {
    TunnelError::new(
        "TUNNEL_CREDENTIAL_ERROR",
        format!("无法{action} Windows SSH 凭据：{error}"),
    )
}

fn write_json_atomically(path: &PathBuf, value: &impl Serialize) -> TunnelResult<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|_| TunnelError::new("TUNNEL_CONFIG_IO", "无法编码隧道配置"))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes).map_err(|error| {
        TunnelError::new("TUNNEL_CONFIG_IO", format!("无法写入隧道配置：{error}"))
    })?;
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        TunnelError::new("TUNNEL_CONFIG_IO", format!("无法保存隧道配置：{error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{SaveServiceConnectionInput, ServiceClient};

    fn isolated_manager(app_data_dir: PathBuf) -> SshTunnelManager {
        let mut manager = SshTunnelManager::new(app_data_dir).expect("manager");
        manager.credential_user = format!("test-private-key-{}", std::process::id());
        manager
    }

    #[test]
    fn validates_endpoint_and_sha256_fingerprint() {
        assert!(validate_endpoint("203.0.113.10", 23).is_ok());
        assert!(validate_endpoint("bad host", 23).is_err());
        assert!(normalize_fingerprint(
            "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned()
        )
        .is_ok());
        assert!(normalize_fingerprint("ssh-ed25519 unknown".to_owned()).is_err());
    }

    #[test]
    #[ignore = "requires a live SSH instance and explicit environment variables"]
    fn opens_a_pinned_native_tunnel_to_the_live_service() {
        let host = std::env::var("ZHIHUA_TEST_SSH_HOST").expect("SSH host");
        let port = std::env::var("ZHIHUA_TEST_SSH_PORT")
            .expect("SSH port")
            .parse()
            .expect("numeric SSH port");
        let fingerprint = std::env::var("ZHIHUA_TEST_SSH_FINGERPRINT").expect("SSH fingerprint");
        let private_key =
            fs::read_to_string(std::env::var("ZHIHUA_TEST_SSH_KEY_PATH").expect("SSH key path"))
                .expect("read SSH private key");
        let directory = tempfile::tempdir().expect("temporary app data");
        let manager = isolated_manager(directory.path().to_path_buf());
        manager
            .save_configuration(SaveTunnelConfigurationInput {
                host,
                port,
                username: "root".to_owned(),
                host_key_fingerprint: fingerprint,
                private_key,
            })
            .expect("save tunnel configuration");
        let result = tauri::async_runtime::block_on(async {
            let status = manager.start().await?;
            let url = status
                .local_url
                .ok_or_else(|| TunnelError::new("TEST", "missing local URL"))?;
            let health = reqwest::get(format!("{url}/api/v1/health"))
                .await
                .map_err(|error| TunnelError::new("TEST", error.to_string()))?
                .json::<serde_json::Value>()
                .await
                .map_err(|error| TunnelError::new("TEST", error.to_string()))?;
            manager.stop().await?;
            Ok::<_, TunnelError>(health)
        })
        .expect("native SSH tunnel contract");
        assert_eq!(result["service"], "zhihua-service");
        assert_eq!(result["version"], "0.3.0");
        let _ = manager
            .credential_entry()
            .and_then(|entry| match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(credential_error("清理", error)),
            });
    }

    #[test]
    #[ignore = "writes the authorized live SSH connection into the current desktop profile"]
    fn provisions_current_desktop_connection() {
        let host = std::env::var("ZHIHUA_TEST_SSH_HOST").expect("SSH host");
        let port = std::env::var("ZHIHUA_TEST_SSH_PORT")
            .expect("SSH port")
            .parse()
            .expect("numeric SSH port");
        let fingerprint = std::env::var("ZHIHUA_TEST_SSH_FINGERPRINT").expect("SSH fingerprint");
        let private_key =
            fs::read_to_string(std::env::var("ZHIHUA_TEST_SSH_KEY_PATH").expect("SSH key path"))
                .expect("read SSH private key");
        let app_data_dir = PathBuf::from(
            std::env::var("ZHIHUA_PROVISION_APP_DATA").expect("desktop app data directory"),
        );
        let instance_id = std::env::var("ZHIHUA_TEST_INSTANCE_ID").expect("instance id");
        let manager = SshTunnelManager::new(app_data_dir.clone()).expect("manager");
        manager
            .save_configuration(SaveTunnelConfigurationInput {
                host,
                port,
                username: "root".to_owned(),
                host_key_fingerprint: fingerprint,
                private_key,
            })
            .expect("save tunnel configuration");
        let service = ServiceClient::new(app_data_dir.clone()).expect("service client");
        let result = tauri::async_runtime::block_on(async {
            let status = manager.start().await?;
            let local_url = status
                .local_url
                .ok_or_else(|| TunnelError::new("TEST", "missing local URL"))?;
            let token = manager.read_service_token().await?;
            service
                .save(SaveServiceConnectionInput {
                    instance_id,
                    base_url: local_url,
                    token,
                })
                .map_err(|error| TunnelError::new(error.code, error.message))?;
            let probe = service
                .probe()
                .await
                .map_err(|error| TunnelError::new(error.code, error.message))?;
            let smoke_path = app_data_dir.join("native-upload-smoke.png");
            fs::write(&smoke_path, b"\x89PNG\r\n\x1a\nnative-tunnel-smoke")
                .map_err(|error| TunnelError::new("TEST", error.to_string()))?;
            let uploaded = service
                .upload_input(&smoke_path.to_string_lossy())
                .await
                .map_err(|error| TunnelError::new(error.code, error.message))?;
            service
                .delete_input(&uploaded.input_id)
                .await
                .map_err(|error| TunnelError::new(error.code, error.message))?;
            let _ = fs::remove_file(smoke_path);
            manager.stop().await?;
            Ok::<_, TunnelError>((probe, uploaded))
        })
        .expect("provision desktop connection");
        assert!(result.0.compatible);
        assert_eq!(result.0.service_version, "0.3.0");
        assert_eq!(result.1.original_filename, "native-upload-smoke.png");
    }
}
