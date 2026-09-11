use futures_util::StreamExt;
use keyring::Entry;
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::RwLock,
    time::Duration,
};
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
};
use tokio_util::io::ReaderStream;

const CREDENTIAL_SERVICE: &str = "cn.zhihua.desktop.zhihua-service";
const CLIENT_VERSION: &str = "0.1.0";
const API_VERSION: &str = "v1";
const WORKFLOW_MANIFEST_VERSION: &str = "zhihua-workflows-2026.09.11";
const MODEL_MANIFEST_VERSION: &str = "public-models-2026.09.08";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_INPUT_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const REQUEST_RETRY_DELAYS_SECONDS: [u64; 6] = [0, 1, 2, 4, 8, 16];

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
    transfer_client: Client,
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
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
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
    pub status_detail: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub result_manifest: Option<ServiceResultManifest>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct ServiceArtifactManifest {
    pub artifact_id: String,
    pub kind: String,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub download_path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct ServiceResultManifest {
    pub schema_version: String,
    pub job_id: String,
    pub workflow_id: String,
    pub prompt_id: Option<String>,
    pub created_at: String,
    pub artifacts: Vec<ServiceArtifactManifest>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInputUpload {
    pub input_id: String,
    pub remote_file: String,
    pub original_filename: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadServiceArtifactInput {
    pub job_id: String,
    pub artifact_id: String,
    pub destination_path: String,
    pub expected_size_bytes: u64,
    pub expected_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceArtifactDownload {
    pub destination_path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub resumed: bool,
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
    status_detail: Option<String>,
    created_at: String,
    updated_at: String,
    result_manifest: Option<ServiceResultManifest>,
}

#[derive(Deserialize)]
struct InputUploadWire {
    input_id: String,
    remote_file: String,
    original_filename: String,
    media_type: String,
    size_bytes: u64,
    sha256: String,
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
            status_detail: job.status_detail,
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
        let transfer_client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| format!("无法初始化知画传输客户端：{error}"))?;
        Ok(Self {
            client,
            transfer_client,
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

    pub fn retarget(&self, base_url: String) -> ServiceResult<ServiceConnectionInfo> {
        let connection = self.connection()?;
        self.save(SaveServiceConnectionInput {
            instance_id: connection.metadata.instance_id,
            base_url,
            token: connection.token,
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
            self.send_with_retry(|| {
                self.authenticated(&connection, Method::POST, "/api/v1/jobs")
                    .json(&JobCreateWire {
                        client_request_id: &input.client_request_id,
                        project_id: &input.project_id,
                        scene_id: &input.scene_id,
                        kind: &input.kind,
                        workflow_id: &input.workflow_id,
                        parameters: &input.parameters,
                    })
            })
            .await?,
        )
        .await?;
        Ok(job.into())
    }

    pub async fn get_job(&self, job_id: &str) -> ServiceResult<ServiceJob> {
        validate_identifier(job_id, "任务 ID")?;
        let connection = self.connection()?;
        let job: JobWire = decode_response(
            self.send_with_retry(|| {
                self.authenticated(&connection, Method::GET, &format!("/api/v1/jobs/{job_id}"))
            })
            .await?,
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

    pub async fn upload_input(&self, source_path: &str) -> ServiceResult<ServiceInputUpload> {
        let path = validate_input_path(source_path)?;
        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|_| service_error("input_unavailable", "无法读取本地参考素材。"))?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_INPUT_BYTES {
            return Err(service_error(
                "input_size_invalid",
                "参考素材为空或超过 4 GB 上限。",
            ));
        }
        validate_local_signature(&path).await?;
        let filename = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| service_error("input_name_invalid", "参考素材文件名无效。"))?;
        let connection = self.connection()?;
        let file = File::open(&path)
            .await
            .map_err(|_| service_error("input_unavailable", "无法打开本地参考素材。"))?;
        let body = reqwest::Body::wrap_stream(ReaderStream::new(file));
        let response = self
            .authenticated_with(
                &self.transfer_client,
                &connection,
                Method::POST,
                "/api/v1/inputs",
            )
            .query(&[("filename", filename)])
            .header(reqwest::header::CONTENT_LENGTH, metadata.len())
            .body(body)
            .send()
            .await
            .map_err(connection_error)?;
        let uploaded: InputUploadWire = decode_response(response).await?;
        Ok(ServiceInputUpload {
            input_id: uploaded.input_id,
            remote_file: uploaded.remote_file,
            original_filename: uploaded.original_filename,
            media_type: uploaded.media_type,
            size_bytes: uploaded.size_bytes,
            sha256: uploaded.sha256,
        })
    }

    pub async fn delete_input(&self, input_id: &str) -> ServiceResult<()> {
        validate_input_id(input_id)?;
        let connection = self.connection()?;
        let response = self
            .authenticated(
                &connection,
                Method::DELETE,
                &format!("/api/v1/inputs/{input_id}"),
            )
            .send()
            .await
            .map_err(connection_error)?;
        ensure_empty_success(response).await
    }

    pub async fn download_artifact(
        &self,
        input: DownloadServiceArtifactInput,
    ) -> ServiceResult<ServiceArtifactDownload> {
        let job_id = validate_identifier(&input.job_id, "任务 ID")?;
        let artifact_id = validate_identifier(&input.artifact_id, "成品 ID")?;
        let expected_sha256 = validate_sha256(&input.expected_sha256)?;
        if input.expected_size_bytes == 0 || input.expected_size_bytes > MAX_ARTIFACT_BYTES {
            return Err(service_error(
                "artifact_size_invalid",
                "成品大小无效或超过 20 GB 上限。",
            ));
        }
        let destination = validate_destination_path(&input.destination_path)?;
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|_| service_error("download_io", "无法创建成品目录。"))?;
        }
        if tokio::fs::metadata(&destination)
            .await
            .map(|value| value.is_file() && value.len() == input.expected_size_bytes)
            .unwrap_or(false)
            && sha256_file(&destination).await? == expected_sha256
        {
            return Ok(ServiceArtifactDownload {
                destination_path: destination.to_string_lossy().into_owned(),
                size_bytes: input.expected_size_bytes,
                sha256: expected_sha256,
                resumed: false,
            });
        }
        let partial = partial_path(&destination);
        let mut existing = tokio::fs::metadata(&partial)
            .await
            .map(|value| value.len())
            .unwrap_or(0);
        if existing > input.expected_size_bytes {
            let _ = tokio::fs::remove_file(&partial).await;
            existing = 0;
        }
        let connection = self.connection()?;
        let path = format!("/api/v1/jobs/{job_id}/artifacts/{artifact_id}");
        let mut request =
            self.authenticated_with(&self.transfer_client, &connection, Method::GET, &path);
        if existing > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={existing}-"));
        }
        let response = request.send().await.map_err(connection_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(response_error(response).await);
        }
        let resumed = existing > 0 && status == StatusCode::PARTIAL_CONTENT;
        if existing > 0 && !resumed {
            existing = 0;
        }
        let mut output = OpenOptions::new()
            .create(true)
            .write(true)
            .append(resumed)
            .truncate(!resumed)
            .open(&partial)
            .await
            .map_err(|_| service_error("download_io", "无法创建成品临时文件。"))?;
        let mut received = existing;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(connection_error)?;
            received = received.saturating_add(chunk.len() as u64);
            if received > input.expected_size_bytes {
                return Err(service_error(
                    "artifact_size_mismatch",
                    "服务返回的成品大于清单记录，已停止下载。",
                ));
            }
            output
                .write_all(&chunk)
                .await
                .map_err(|_| service_error("download_io", "写入成品文件失败。"))?;
        }
        output
            .flush()
            .await
            .map_err(|_| service_error("download_io", "刷新成品文件失败。"))?;
        drop(output);
        if received != input.expected_size_bytes {
            return Err(service_error(
                "artifact_incomplete",
                "成品下载未完成，可稍后继续。",
            ));
        }
        let actual_sha256 = sha256_file(&partial).await?;
        if actual_sha256 != expected_sha256 {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(service_error(
                "artifact_checksum_mismatch",
                "成品校验失败，临时文件已删除。",
            ));
        }
        if destination.exists() {
            tokio::fs::remove_file(&destination)
                .await
                .map_err(|_| service_error("download_io", "无法替换已有成品。"))?;
        }
        tokio::fs::rename(&partial, &destination)
            .await
            .map_err(|_| service_error("download_io", "无法完成成品文件保存。"))?;
        Ok(ServiceArtifactDownload {
            destination_path: destination.to_string_lossy().into_owned(),
            size_bytes: received,
            sha256: actual_sha256,
            resumed,
        })
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
        self.authenticated_with(&self.client, connection, method, path)
    }

    fn authenticated_with(
        &self,
        client: &Client,
        connection: &ActiveConnection,
        method: Method,
        path: &str,
    ) -> reqwest::RequestBuilder {
        client
            .request(method, format!("{}{path}", connection.metadata.base_url))
            .bearer_auth(&connection.token)
            .header("X-Zhihua-API-Version", API_VERSION)
            .header("X-Zhihua-Client-Version", CLIENT_VERSION)
    }

    async fn send_with_retry<F>(&self, mut build: F) -> ServiceResult<Response>
    where
        F: FnMut() -> RequestBuilder,
    {
        for (attempt, delay_seconds) in REQUEST_RETRY_DELAYS_SECONDS.iter().enumerate() {
            if *delay_seconds > 0 {
                tokio::time::sleep(retry_delay(*delay_seconds)).await;
            }
            match build().send().await {
                Ok(response)
                    if transient_status(response.status())
                        && attempt + 1 < REQUEST_RETRY_DELAYS_SECONDS.len() =>
                {
                    continue;
                }
                Ok(response) => return Ok(response),
                Err(error) => {
                    let error = connection_error(error);
                    if transient_error(&error) && attempt + 1 < REQUEST_RETRY_DELAYS_SECONDS.len() {
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        Err(service_error(
            "request_failed",
            "知画服务请求在连接恢复后仍未成功。",
        ))
    }
}

fn transient_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn transient_error(error: &ServiceConnectionError) -> bool {
    matches!(error.code, "timeout" | "unreachable" | "request_failed")
}

fn retry_delay(seconds: u64) -> Duration {
    if cfg!(test) {
        Duration::ZERO
    } else {
        Duration::from_secs(seconds)
    }
}

async fn ensure_empty_success(response: Response) -> ServiceResult<()> {
    if response.status().is_success() {
        return Ok(());
    }
    Err(response_error(response).await)
}

async fn response_error(response: Response) -> ServiceConnectionError {
    let status = response.status();
    let bytes = response.bytes().await.unwrap_or_default();
    let code = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| {
            value
                .pointer("/detail/code")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown_error".to_owned());
    service_error(
        "http_status",
        &format!("知画服务返回 HTTP {}：{code}。", status.as_u16()),
    )
}

fn validate_input_path(value: &str) -> ServiceResult<PathBuf> {
    let path = PathBuf::from(value.trim());
    if !path.is_absolute() {
        return Err(service_error(
            "input_path_invalid",
            "参考素材必须使用本机绝对路径。",
        ));
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "mp4" | "mov" | "webm"
    ) {
        return Err(service_error(
            "input_type_not_supported",
            "参考素材格式不受支持。",
        ));
    }
    Ok(path)
}

fn validate_destination_path(value: &str) -> ServiceResult<PathBuf> {
    let path = PathBuf::from(value.trim());
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(service_error(
            "download_path_invalid",
            "成品保存路径必须是本机绝对文件路径。",
        ));
    }
    Ok(path)
}

fn partial_path(destination: &Path) -> PathBuf {
    let filename = destination
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("artifact");
    destination.with_file_name(format!(".{filename}.zhihua-part"))
}

fn validate_input_id(value: &str) -> ServiceResult<()> {
    let Some((stem, extension)) = value.rsplit_once('.') else {
        return Err(service_error("input_id_invalid", "远端素材 ID 无效。"));
    };
    let valid_extension = matches!(
        extension,
        "jpg" | "jpeg" | "png" | "webp" | "mp4" | "mov" | "webm"
    );
    if stem.len() != 32 || !stem.bytes().all(|value| value.is_ascii_hexdigit()) || !valid_extension
    {
        return Err(service_error("input_id_invalid", "远端素材 ID 无效。"));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> ServiceResult<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() != 64 || !value.bytes().all(|character| character.is_ascii_hexdigit()) {
        return Err(service_error(
            "artifact_checksum_invalid",
            "成品 SHA-256 无效。",
        ));
    }
    Ok(value)
}

async fn validate_local_signature(path: &Path) -> ServiceResult<()> {
    let mut file = File::open(path)
        .await
        .map_err(|_| service_error("input_unavailable", "无法读取本地参考素材。"))?;
    let mut header = [0_u8; 16];
    let length = file
        .read(&mut header)
        .await
        .map_err(|_| service_error("input_unavailable", "无法读取本地参考素材。"))?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let valid = match extension.as_str() {
        "jpg" | "jpeg" => header[..length].starts_with(&[0xff, 0xd8, 0xff]),
        "png" => header[..length].starts_with(b"\x89PNG\r\n\x1a\n"),
        "webp" => length >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP",
        "mp4" | "mov" => length >= 12 && &header[4..8] == b"ftyp",
        "webm" => header[..length].starts_with(&[0x1a, 0x45, 0xdf, 0xa3]),
        _ => false,
    };
    if !valid {
        return Err(service_error(
            "input_content_invalid",
            "参考素材内容与扩展名不匹配。",
        ));
    }
    Ok(())
}

async fn sha256_file(path: &Path) -> ServiceResult<String> {
    let mut file = File::open(path)
        .await
        .map_err(|_| service_error("download_io", "无法校验成品文件。"))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let length = file
            .read(&mut buffer)
            .await
            .map_err(|_| service_error("download_io", "无法校验成品文件。"))?;
        if length == 0 {
            break;
        }
        digest.update(&buffer[..length]);
    }
    Ok(format!("{:x}", digest.finalize()))
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
        "image_generation"
            | "image_edit"
            | "video_candidate"
            | "video_reference_remake"
            | "video_upscale"
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

    fn connected_client(base_url: String) -> ServiceClient {
        let client = client();
        *client.active.write().expect("active connection") = Some(ActiveConnection {
            metadata: ConnectionMetadata {
                instance_id: "test-instance".to_owned(),
                base_url,
            },
            token: "a".repeat(32),
        });
        client
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
    fn decodes_service_result_manifest_and_serializes_it_for_the_frontend() {
        let wire: JobWire = serde_json::from_value(serde_json::json!({
            "id": "job-1",
            "client_request_id": "request-1",
            "project_id": "project-1",
            "scene_id": "scene-1",
            "kind": "video_candidate",
            "workflow_id": "h3-t2v-turbo-v1",
            "status": "completed",
            "prompt_id": "prompt-1",
            "progress": 1.0,
            "error_code": null,
            "error_message": null,
            "status_detail": "done",
            "created_at": "2026-09-11T00:00:00Z",
            "updated_at": "2026-09-11T00:01:00Z",
            "result_manifest": {
                "schema_version": "1",
                "job_id": "job-1",
                "workflow_id": "h3-t2v-turbo-v1",
                "prompt_id": "prompt-1",
                "created_at": "2026-09-11T00:01:00Z",
                "artifacts": [{
                    "artifact_id": "video-0",
                    "kind": "video",
                    "filename": "clip.mp4",
                    "media_type": "video/mp4",
                    "size_bytes": 42,
                    "sha256": "a".repeat(64),
                    "download_path": "/api/v1/jobs/job-1/artifacts/video-0"
                }]
            }
        }))
        .expect("decode completed job");

        let frontend = serde_json::to_value(ServiceJob::from(wire)).expect("serialize job");
        assert_eq!(frontend["resultManifest"]["jobId"], "job-1");
        assert_eq!(frontend["resultManifest"]["artifacts"][0]["sizeBytes"], 42);
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
            workflow_id: "h3-flf2v-turbo-v1".to_owned(),
            parameters: serde_json::json!({}),
        };
        assert_eq!(
            validate_job_input(&input).expect_err("invalid kind").code,
            "invalid_job_kind"
        );
    }

    #[test]
    fn retries_idempotent_job_reads_after_a_transient_gateway_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        thread::spawn(move || {
            for (index, stream) in listener.incoming().take(2).enumerate() {
                let mut stream = stream.expect("accept request");
                let mut request = [0_u8; 4096];
                let read = stream.read(&mut request).expect("read request");
                assert!(String::from_utf8_lossy(&request[..read])
                    .starts_with("GET /api/v1/jobs/job-1 HTTP/1.1"));
                let (status, body) = if index == 0 {
                    (
                        "502 Bad Gateway",
                        r#"{"detail":{"code":"tunnel_reconnecting"}}"#,
                    )
                } else {
                    (
                        "200 OK",
                        r#"{"id":"job-1","client_request_id":"request-1","project_id":"project-1","scene_id":"scene-1","kind":"video_candidate","workflow_id":"h3-t2v-turbo-v1","status":"running","prompt_id":"prompt-1","progress":0.5,"error_code":null,"error_message":null,"status_detail":"ComfyUI is executing the prompt","created_at":"2026-09-11T00:00:00Z","updated_at":"2026-09-11T00:00:01Z","result_manifest":null}"#,
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).expect("response");
            }
        });
        let client = connected_client(format!("http://{address}"));
        let job = tauri::async_runtime::block_on(client.get_job("job-1"))
            .expect("job read should recover");
        assert_eq!(job.id, "job-1");
        assert_eq!(job.status, "running");
    }

    #[test]
    fn resumes_and_verifies_artifact_download() {
        let content = b"0123456789";
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).expect("read request");
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with("GET /api/v1/jobs/job-1/artifacts/video-0 HTTP/1.1"));
            assert!(request.to_ascii_lowercase().contains("range: bytes=3-"));
            let remaining = &content[3..];
            let response = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nContent-Range: bytes 3-9/10\r\nConnection: close\r\n\r\n",
                remaining.len()
            );
            stream.write_all(response.as_bytes()).expect("headers");
            stream.write_all(remaining).expect("content");
        });
        let directory = TempDir::new().expect("download directory");
        let destination = directory.path().join("result.mp4");
        fs::write(partial_path(&destination), &content[..3]).expect("partial file");
        let expected_sha256 = format!("{:x}", Sha256::digest(content));
        let client = connected_client(format!("http://{address}"));
        let downloaded = tauri::async_runtime::block_on(client.download_artifact(
            DownloadServiceArtifactInput {
                job_id: "job-1".to_owned(),
                artifact_id: "video-0".to_owned(),
                destination_path: destination.to_string_lossy().into_owned(),
                expected_size_bytes: content.len() as u64,
                expected_sha256: expected_sha256.clone(),
            },
        ))
        .expect("resumed download");
        assert!(downloaded.resumed);
        assert_eq!(downloaded.sha256, expected_sha256);
        assert_eq!(fs::read(destination).expect("downloaded file"), content);
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
                client_request_id: "rust-client-live-smoke-t2v-20260911-v3".to_owned(),
                project_id: "rust-client-project".to_owned(),
                scene_id: "rust-client-scene".to_owned(),
                kind: "video_candidate".to_owned(),
                workflow_id: "h3-t2v-turbo-v1".to_owned(),
                parameters: serde_json::json!({
                    "prompt": "雷电形成过程",
                    "durationSec": 5,
                    "width": 1344,
                    "height": 768,
                    "visibleWidth": 1344,
                    "visibleHeight": 756,
                    "cropX": 0,
                    "cropY": 6,
                    "length": 124,
                    "discardH3Audio": true,
                    "outputPrefix": "video/zhihua/live-contract/rust-client-v3"
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
        assert_eq!(probe.service_version, "0.4.0");
        assert!(probe.workflows.iter().any(|item| item == "h3-t2v-turbo-v1"));
        assert_eq!(probe.available_workflows.len(), 11);
        assert!(probe
            .available_workflows
            .iter()
            .any(|item| item == "h3-t2v-turbo-v1"));
        assert_eq!(first.id, second.id);
        assert_eq!(fetched.id, first.id);
        assert!(!first.id.is_empty());
    }
}
