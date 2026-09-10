use keyring::Entry;
use reqwest::{Client, Method, Response, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{fs, path::PathBuf, sync::RwLock, time::Duration};

const CREDENTIAL_SERVICE: &str = "cn.zhihua.desktop.zhihua-service";
const CLIENT_VERSION: &str = "0.1.0";
const API_VERSION: &str = "v1";
const WORKFLOW_MANIFEST_VERSION: &str = "h3-workflows-2026.09.10";
const MODEL_MANIFEST_VERSION: &str = "public-models-2026.09.08";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct ServiceConnectionError {
    pub code: &'static str,
    pub message: String,
}

type ServiceResult<T> = Result<T, ServiceConnectionError>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionMetadata {
    instance_id: String,
    base_url: String,
}

#[derive(Clone)]
struct ActiveConnection {
    metadata: ConnectionMetadata,
    token: String,
}

pub struct ServiceClient {
    client: Client,
    metadata_path: PathBuf,
    active: RwLock<Option<ActiveConnection>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveServiceConnectionInput {
    pub instance_id: String,
    pub base_url: String,
    pub token: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceConnectionInfo {
    pub configured: bool,
    pub instance_id: Option<String>,
    pub base_url: Option<String>,
    pub credential_stored: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceConnectionResult {
    pub base_url: String,
    pub service: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceProbe {
    pub reachable: bool,
    pub compatible: bool,
    pub service_version: String,
    pub api_version: Option<String>,
    pub workflow_manifest_version: String,
    pub model_manifest_version: String,
    pub workflows: Vec<String>,
    pub available_workflows: Vec<String>,
    pub comfyui_connected: bool,
    pub comfyui_ready: bool,
    pub queue_active: u64,
    pub queue_queued: u64,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitServiceJobInput {
    pub client_request_id: String,
    pub project_id: String,
    pub scene_id: String,
    pub kind: String,
    pub workflow_id: String,
    #[serde(default)]
    pub parameters: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceJob {
    pub id: String,
    pub client_request_id: String,
    pub project_id: String,
    pub scene_id: String,
    pub kind: String,
    pub workflow_id: String,
    pub status: String,
    pub prompt_id: Option<String>,
    pub progress: f64,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub result_manifest: Option<Value>,
}

#[derive(Deserialize)]
struct HealthWire {
    status: String,
    service: String,
    version: String,
}

#[derive(Serialize)]
struct HandshakeRequestWire<'a> {
    client_version: &'a str,
    supported_api_versions: [&'a str; 1],
    workflow_manifest_versions: [&'a str; 1],
    model_manifest_versions: [&'a str; 1],
}

#[derive(Deserialize)]
struct HandshakeWire {
    compatible: bool,
    reasons: Vec<String>,
    service_version: String,
    selected_api_version: Option<String>,
    workflow_manifest_version: String,
    model_manifest_version: String,
}

#[derive(Deserialize)]
struct ComfyUiWire {
    connected: bool,
    ready: bool,
    detail: Option<String>,
}

#[derive(Deserialize)]
struct QueueWire {
    active: u64,
    queued: u64,
}

#[derive(Deserialize)]
struct CapabilitiesWire {
    workflows: Vec<String>,
    available_workflows: Vec<String>,
}

#[derive(Serialize)]
struct JobCreateWire<'a> {
    client_request_id: &'a str,
    project_id: &'a str,
    scene_id: &'a str,
    kind: &'a str,
    workflow_id: &'a str,
    parameters: &'a Value,
}

#[derive(Deserialize)]
struct JobWire {
    id: String,
    client_request_id: String,
    project_id: String,
    scene_id: String,
    kind: String,
    workflow_id: String,
    status: String,
    prompt_id: Option<String>,
    progress: f64,
    error_code: Option<String>,
    error_message: Option<String>,
    created_at: String,
    updated_at: String,
    result_manifest: Option<Value>,
}

impl From<JobWire> for ServiceJob {
    fn from(job: JobWire) -> Self {
        Self {
            id: job.id,
            client_request_id: job.client_request_id,
            project_id: job.project_id,
            scene_id: job.scene_id,
            kind: job.kind,
            workflow_id: job.workflow_id,
            status: job.status,
            prompt_id: job.prompt_id,
            progress: job.progress,
            error_code: job.error_code,
            error_message: job.error_message,
            created_at: job.created_at,
            updated_at: job.updated_at,
            result_manifest: job.result_manifest,
        }
    }
}

impl ServiceClient {
    pub fn new(app_data_dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&app_data_dir).map_err(|error| error.to_string())?;
        let metadata_path = app_data_dir.join("service-connection.json");
        let active = load_metadata(&metadata_path)
            .ok()
            .flatten()
            .and_then(|metadata| {
                load_token(&metadata.instance_id)
                    .ok()
                    .map(|token| ActiveConnection { metadata, token })
            });
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|error| format!("无法初始化知画服务客户端：{error}"))?;
        Ok(Self {
            client,
            metadata_path,
            active: RwLock::new(active),
        })
    }

    pub fn info(&self) -> ServiceResult<ServiceConnectionInfo> {
        let active = self.active.read().map_err(lock_error)?;
        if let Some(connection) = active.as_ref() {
            return Ok(ServiceConnectionInfo {
                configured: true,
                instance_id: Some(connection.metadata.instance_id.clone()),
                base_url: Some(connection.metadata.base_url.clone()),
                credential_stored: true,
            });
        }
        let metadata = load_metadata(&self.metadata_path)?;
        Ok(ServiceConnectionInfo {
            configured: metadata.is_some(),
            instance_id: metadata.as_ref().map(|value| value.instance_id.clone()),
            base_url: metadata.as_ref().map(|value| value.base_url.clone()),
            credential_stored: false,
        })
    }

    pub fn save(&self, input: SaveServiceConnectionInput) -> ServiceResult<ServiceConnectionInfo> {
        let instance_id = validate_identifier(&input.instance_id, "实例 ID")?;
        let base_url = validate_base_url(&input.base_url)?;
        if !(32..=512).contains(&input.token.chars().count()) {
            return Err(service_error(
                "invalid_token",
                "知画服务令牌长度必须为 32～512 个字符。",
            ));
        }
        let metadata = ConnectionMetadata {
            instance_id,
            base_url,
        };
        save_token(&metadata.instance_id, &input.token)?;
        let encoded = serde_json::to_vec_pretty(&metadata)
            .map_err(|_| service_error("metadata_failed", "无法编码知画服务连接信息。"))?;
        if fs::write(&self.metadata_path, encoded).is_err() {
            let _ = delete_token(&metadata.instance_id);
            return Err(service_error(
                "metadata_failed",
                "无法保存知画服务连接信息。",
            ));
        }
        *self.active.write().map_err(lock_error)? = Some(ActiveConnection {
            metadata: metadata.clone(),
            token: input.token,
        });
        Ok(ServiceConnectionInfo {
            configured: true,
            instance_id: Some(metadata.instance_id),
            base_url: Some(metadata.base_url),
            credential_stored: true,
        })
    }

    pub fn clear(&self) -> ServiceResult<()> {
        if let Some(metadata) = load_metadata(&self.metadata_path)? {
            let _ = delete_token(&metadata.instance_id);
        }
        if self.metadata_path.exists() {
            fs::remove_file(&self.metadata_path)
                .map_err(|_| service_error("metadata_failed", "无法删除知画服务连接信息。"))?;
        }
        *self.active.write().map_err(lock_error)? = None;
        Ok(())
    }

    pub async fn test_connection(&self, base_url: &str) -> ServiceResult<ServiceConnectionResult> {
        let base_url = validate_base_url(base_url)?;
        let health: HealthWire = decode_response(
            self.client
                .get(format!("{base_url}/api/v1/health"))
                .send()
                .await
                .map_err(connection_error)?,
        )
        .await?;
        validate_health(&health)?;
        Ok(ServiceConnectionResult {
            base_url,
            service: health.service,
            version: health.version,
        })
    }

    pub async fn probe(&self) -> ServiceResult<ServiceProbe> {
        let connection = self.connection()?;
        let base_url = &connection.metadata.base_url;
        let health: HealthWire = decode_response(
            self.client
                .get(format!("{base_url}/api/v1/health"))
                .send()
                .await
                .map_err(connection_error)?,
        )
        .await?;
        validate_health(&health)?;
        let handshake: HandshakeWire = decode_response(
            self.client
                .post(format!("{base_url}/api/v1/handshake"))
                .bearer_auth(&connection.token)
                .json(&HandshakeRequestWire {
                    client_version: CLIENT_VERSION,
                    supported_api_versions: [API_VERSION],
                    workflow_manifest_versions: [WORKFLOW_MANIFEST_VERSION],
                    model_manifest_versions: [MODEL_MANIFEST_VERSION],
                })
                .send()
                .await
                .map_err(connection_error)?,
        )
        .await?;
        let comfyui: ComfyUiWire = decode_response(
            self.client
                .get(format!("{base_url}/api/v1/comfyui/status"))
                .send()
                .await
                .map_err(connection_error)?,
        )
        .await?;
        let capabilities: CapabilitiesWire = decode_response(
            self.client
                .get(format!("{base_url}/api/v1/capabilities"))
                .send()
                .await
                .map_err(connection_error)?,
        )
        .await?;
        let queue: QueueWire = decode_response(
            self.authenticated(&connection, Method::GET, "/api/v1/jobs/queue")
                .send()
                .await
                .map_err(connection_error)?,
        )
        .await?;
        let detail = if !handshake.compatible {
            format!("版本握手不兼容：{}", handshake.reasons.join(", "))
        } else if let Some(detail) = comfyui.detail {
            detail
        } else if comfyui.ready {
            "知画服务和 ComfyUI 已就绪。".to_owned()
        } else {
            "知画服务可用，ComfyUI 等待 GPU 启动。".to_owned()
        };
        Ok(ServiceProbe {
            reachable: true,
            compatible: handshake.compatible,
            service_version: handshake.service_version,
            api_version: handshake.selected_api_version,
            workflow_manifest_version: handshake.workflow_manifest_version,
            model_manifest_version: handshake.model_manifest_version,
            workflows: capabilities.workflows,
            available_workflows: capabilities.available_workflows,
            comfyui_connected: comfyui.connected,
            comfyui_ready: comfyui.ready,
            queue_active: queue.active,
            queue_queued: queue.queued,
            detail,
        })
    }

    pub async fn submit_job(&self, input: SubmitServiceJobInput) -> ServiceResult<ServiceJob> {
        validate_job_input(&input)?;
        let connection = self.connection()?;
        let job: JobWire = decode_response(
            self.authenticated(&connection, Method::POST, "/api/v1/jobs")
                .json(&JobCreateWire {
                    client_request_id: &input.client_request_id,
                    project_id: &input.project_id,
                    scene_id: &input.scene_id,
                    kind: &input.kind,
                    workflow_id: &input.workflow_id,
                    parameters: &input.parameters,
                })
                .send()
                .await
                .map_err(connection_error)?,
        )
        .await?;
        Ok(job.into())
    }

    pub async fn get_job(&self, job_id: &str) -> ServiceResult<ServiceJob> {
        validate_identifier(job_id, "任务 ID")?;
        let connection = self.connection()?;
        let job: JobWire = decode_response(
            self.authenticated(&connection, Method::GET, &format!("/api/v1/jobs/{job_id}"))
                .send()
                .await
                .map_err(connection_error)?,
        )
        .await?;
        Ok(job.into())
    }

    pub async fn cancel_job(&self, job_id: &str) -> ServiceResult<ServiceJob> {
        validate_identifier(job_id, "任务 ID")?;
        let connection = self.connection()?;
        let _: Value = decode_response(
            self.authenticated(
                &connection,
                Method::POST,
                &format!("/api/v1/jobs/{job_id}/cancel"),
            )
            .send()
            .await
            .map_err(connection_error)?,
        )
        .await?;
        self.get_job(job_id).await
    }

    fn connection(&self) -> ServiceResult<ActiveConnection> {
        self.active
            .read()
            .map_err(lock_error)?
            .clone()
            .ok_or_else(|| service_error("not_configured", "尚未配置知画服务，或系统凭据已丢失。"))
    }

    fn authenticated(
        &self,
        connection: &ActiveConnection,
        method: Method,
        path: &str,
    ) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("{}{path}", connection.metadata.base_url))
            .bearer_auth(&connection.token)
            .header("X-Zhihua-API-Version", API_VERSION)
            .header("X-Zhihua-Client-Version", CLIENT_VERSION)
    }
}

fn validate_health(health: &HealthWire) -> ServiceResult<()> {
    if health.status != "ok" || health.service != "zhihua-service" || health.version.is_empty() {
        return Err(service_error(
            "incompatible_service",
            "目标地址不是兼容的知画服务。",
        ));
    }
    Ok(())
}

fn validate_base_url(value: &str) -> ServiceResult<String> {
    let mut url = Url::parse(value.trim()).map_err(|_| invalid_address())?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "http" || !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err(service_error(
            "unsafe_address",
            "当前版本仅允许通过本机 SSH 端口转发连接知画服务。",
        ));
    }
    if url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(invalid_address());
    }
    url.set_path("");
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn validate_identifier(value: &str, label: &str) -> ServiceResult<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".:_-".contains(character))
    {
        return Err(service_error(
            "invalid_identifier",
            &format!("{label}格式无效。"),
        ));
    }
    Ok(value.to_owned())
}

fn validate_job_input(input: &SubmitServiceJobInput) -> ServiceResult<()> {
    validate_identifier(&input.client_request_id, "客户端请求 ID")?;
    validate_identifier(&input.project_id, "项目 ID")?;
    validate_identifier(&input.scene_id, "分镜 ID")?;
    validate_identifier(&input.workflow_id, "工作流 ID")?;
    if !matches!(
        input.kind.as_str(),
        "video_candidate" | "video_reference_remake" | "video_upscale"
    ) {
        return Err(service_error("invalid_job_kind", "任务类型不受支持。"));
    }
    if !input.parameters.is_object() {
        return Err(service_error("invalid_parameters", "任务参数必须是对象。"));
    }
    Ok(())
}

fn credential_entry(instance_id: &str) -> ServiceResult<Entry> {
    Entry::new(CREDENTIAL_SERVICE, instance_id)
        .map_err(|_| service_error("credential_error", "无法访问 Windows 系统凭据库。"))
}

fn save_token(instance_id: &str, token: &str) -> ServiceResult<()> {
    credential_entry(instance_id)?
        .set_password(token)
        .map_err(|_| service_error("credential_error", "无法保存知画服务凭据。"))
}

fn load_token(instance_id: &str) -> ServiceResult<String> {
    credential_entry(instance_id)?
        .get_password()
        .map_err(|_| service_error("credential_error", "无法读取知画服务凭据。"))
}

fn delete_token(instance_id: &str) -> ServiceResult<()> {
    credential_entry(instance_id)?
        .delete_credential()
        .map_err(|_| service_error("credential_error", "无法删除知画服务凭据。"))
}

fn load_metadata(path: &PathBuf) -> ServiceResult<Option<ConnectionMetadata>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)
        .map_err(|_| service_error("metadata_failed", "无法读取知画服务连接信息。"))?;
    let metadata: ConnectionMetadata = serde_json::from_slice(&bytes)
        .map_err(|_| service_error("metadata_failed", "知画服务连接信息已损坏。"))?;
    validate_identifier(&metadata.instance_id, "实例 ID")?;
    validate_base_url(&metadata.base_url)?;
    Ok(Some(metadata))
}

fn invalid_address() -> ServiceConnectionError {
    service_error(
        "invalid_address",
        "请输入本机 SSH 端口转发地址，例如 http://127.0.0.1:18000。",
    )
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> ServiceConnectionError {
    service_error("state_error", "知画服务连接状态不可用。")
}

fn connection_error(error: reqwest::Error) -> ServiceConnectionError {
    if error.is_timeout() {
        service_error(
            "timeout",
            "连接知画服务超时，请检查优云智算实例和 SSH 端口转发。",
        )
    } else if error.is_connect() {
        service_error(
            "unreachable",
            "无法连接知画服务，请确认实例和 SSH 端口转发已启动。",
        )
    } else {
        service_error("request_failed", "知画服务请求失败。")
    }
}

async fn decode_response<T: DeserializeOwned>(response: Response) -> ServiceResult<T> {
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|_| service_error("invalid_response", "无法读取知画服务响应。"))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(service_error(
            "response_too_large",
            "知画服务响应异常，已停止读取。",
        ));
    }
    if !status.is_success() {
        let code = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/detail/code")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "unknown_error".to_owned());
        return Err(service_error(
            "http_status",
            &format!("知画服务返回 HTTP {}：{code}。", status.as_u16()),
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| service_error("invalid_response", "知画服务响应格式无效。"))
}

fn service_error(code: &'static str, message: &str) -> ServiceConnectionError {
    ServiceConnectionError {
        code,
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use tempfile::TempDir;

    fn client() -> ServiceClient {
        let directory = TempDir::new().expect("temp directory");
        ServiceClient::new(directory.keep()).expect("service client")
    }

    fn mock_server(status: &str, body: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let status = status.to_owned();
        let body = body.to_owned();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 1024];
            let read = stream.read(&mut request).expect("read request");
            assert!(String::from_utf8_lossy(&request[..read])
                .starts_with("GET /api/v1/health HTTP/1.1"));
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).expect("response");
        });
        format!("http://{address}")
    }

    #[test]
    fn accepts_only_plain_loopback_tunnel_urls() {
        assert_eq!(
            validate_base_url(" http://127.0.0.1:18000/ ").expect("loopback URL"),
            "http://127.0.0.1:18000"
        );
        for value in [
            "https://127.0.0.1:18000",
            "http://example.com:18000",
            "http://user:pass@127.0.0.1:18000",
            "http://127.0.0.1:18000/api/v1",
            "http://127.0.0.1",
        ] {
            assert!(validate_base_url(value).is_err(), "{value}");
        }
    }

    #[test]
    fn reads_health_contract_and_version() {
        let base_url = mock_server(
            "200 OK",
            r#"{"status":"ok","service":"zhihua-service","version":"0.2.0"}"#,
        );
        let result = tauri::async_runtime::block_on(client().test_connection(&base_url))
            .expect("healthy service");
        assert_eq!(result.service, "zhihua-service");
        assert_eq!(result.version, "0.2.0");
    }

    #[test]
    fn rejects_incompatible_health_payload() {
        let base_url = mock_server(
            "200 OK",
            r#"{"status":"ok","service":"another-service","version":"1.0.0"}"#,
        );
        let error = tauri::async_runtime::block_on(client().test_connection(&base_url))
            .expect_err("incompatible response");
        assert_eq!(error.code, "incompatible_service");
    }

    #[test]
    fn rejects_invalid_job_kinds() {
        let input = SubmitServiceJobInput {
            client_request_id: "request-1".to_owned(),
            project_id: "project-1".to_owned(),
            scene_id: "scene-1".to_owned(),
            kind: "arbitrary_command".to_owned(),
            workflow_id: "h3-fl2v-turbo-v1".to_owned(),
            parameters: serde_json::json!({}),
        };
        assert_eq!(
            validate_job_input(&input).expect_err("invalid kind").code,
            "invalid_job_kind"
        );
    }

    #[test]
    #[ignore = "requires an SSH tunnel and ZHIHUA_TEST_SERVICE_TOKEN"]
    fn connects_to_live_secure_service_and_recovers_idempotent_job() {
        let base_url = std::env::var("ZHIHUA_TEST_SERVICE_URL")
            .expect("set ZHIHUA_TEST_SERVICE_URL for live contract test");
        let token = std::env::var("ZHIHUA_TEST_SERVICE_TOKEN")
            .expect("set ZHIHUA_TEST_SERVICE_TOKEN for live contract test");
        let client = client();
        client
            .save(SaveServiceConnectionInput {
                instance_id: "zhihua-live-contract-test".to_owned(),
                base_url,
                token,
            })
            .expect("save temporary credential");

        let result: ServiceResult<_> = tauri::async_runtime::block_on(async {
            let probe = client.probe().await?;
            let request = SubmitServiceJobInput {
                client_request_id: "rust-client-live-smoke-t2v-20260910".to_owned(),
                project_id: "rust-client-project".to_owned(),
                scene_id: "rust-client-scene".to_owned(),
                kind: "video_candidate".to_owned(),
                workflow_id: "h3-t2v-turbo-v1".to_owned(),
                parameters: serde_json::json!({
                    "prompt": "雷电形成过程",
                    "durationSec": 5
                }),
            };
            let first = client.submit_job(request.clone()).await?;
            let second = client.submit_job(request).await?;
            let fetched = client.get_job(&first.id).await?;
            Ok((probe, first, second, fetched))
        });

        client.clear().expect("clear temporary credential");
        let (probe, first, second, fetched) = result.expect("run live service contract");
        assert!(probe.compatible);
        assert_eq!(probe.api_version.as_deref(), Some("v1"));
        assert_eq!(probe.service_version, "0.2.0");
        assert!(probe.workflows.iter().any(|item| item == "h3-t2v-turbo-v1"));
        assert!(probe.available_workflows.is_empty());
        assert_eq!(first.id, second.id);
        assert_eq!(fetched.id, first.id);
        assert!(!first.id.is_empty());
    }
}
