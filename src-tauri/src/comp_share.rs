use keyring::Entry;
use reqwest::{Client, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{collections::BTreeMap, fs, path::PathBuf, sync::RwLock, time::Duration};

const API_BASE_URL: &str = "https://api.compshare.cn";
const CREDENTIAL_SERVICE: &str = "cn.zhihua.desktop.compshare";
const CREDENTIAL_USER: &str = "api-credentials";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(6);

type CompShareResult<T> = Result<T, CompShareError>;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompShareError {
    pub code: String,
    pub message: String,
    pub ret_code: Option<i64>,
    pub request_uuid: Option<String>,
}

impl CompShareError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            ret_code: None,
            request_uuid: None,
        }
    }

    fn api(ret_code: i64, message: String, request_uuid: Option<String>) -> Self {
        Self {
            code: "COMPSHARE_API_ERROR".to_string(),
            message: if message.trim().is_empty() {
                format!("优云智算接口返回错误码 {ret_code}")
            } else {
                message
            },
            ret_code: Some(ret_code),
            request_uuid,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ApiCredentials {
    public_key: String,
    private_key: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompShareMetadata {
    bound_instance: Option<BoundInstance>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BoundInstance {
    instance_id: String,
    region: String,
    zone: String,
    project_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveCompShareCredentialsInput {
    pub public_key: String,
    pub private_key: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListCompShareInstancesInput {
    pub region: Option<String>,
    pub zone: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindCompShareInstanceInput {
    pub instance_id: String,
    pub region: String,
    pub zone: String,
    pub project_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCompShareStopSchedulerInput {
    pub stop_time: i64,
    pub project_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CompSharePowerState {
    Running,
    Stopped,
    Starting,
    Stopping,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CompShareRunningMode {
    Gpu,
    NoGpu,
    Stopped,
    Transitioning,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CompShareStartMode {
    Gpu,
    NoGpu,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompShareConfiguration {
    pub credentials_stored: bool,
    pub bound_instance_id: Option<String>,
    pub region: Option<String>,
    pub zone: Option<String>,
    pub project_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompShareBalance {
    pub amount: Option<String>,
    pub amount_available: Option<String>,
    pub amount_credit: Option<String>,
    pub amount_free: Option<String>,
    pub amount_freeze: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompShareConnectionTest {
    pub connected: bool,
    pub balance: CompShareBalance,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompShareInstance {
    pub instance_id: String,
    pub name: Option<String>,
    pub region: String,
    pub zone: String,
    pub state: CompSharePowerState,
    pub raw_state: String,
    pub running_mode: CompShareRunningMode,
    pub cpu: Option<u32>,
    pub memory_mb: Option<u64>,
    pub gpu_count: Option<u32>,
    pub gpu_type: Option<String>,
    pub support_without_gpu_start: bool,
    pub ssh_login_command: Option<String>,
    pub start_time: Option<i64>,
    pub stop_time: Option<i64>,
    pub release_time: Option<i64>,
    pub stop_scheduler_time: Option<i64>,
    pub instance_price: Option<f64>,
    pub project_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompShareActionResult {
    pub request_sent: bool,
    pub requested_mode: Option<CompShareStartMode>,
    pub instance: CompShareInstance,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompShareSchedulerResult {
    pub stop_time: i64,
    pub instance: CompShareInstance,
}

struct CompShareApi {
    client: Client,
    base_url: Url,
}

impl CompShareApi {
    fn new(base_url: &str) -> CompShareResult<Self> {
        let base_url = Url::parse(base_url).map_err(|error| {
            CompShareError::new("INVALID_API_URL", format!("优云智算 API 地址无效：{error}"))
        })?;
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent(format!("zhihua-desktop/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| {
                CompShareError::new(
                    "HTTP_CLIENT_INIT_FAILED",
                    format!("无法初始化优云智算网络客户端：{error}"),
                )
            })?;
        Ok(Self { client, base_url })
    }

    async fn invoke<T: DeserializeOwned>(
        &self,
        credentials: &ApiCredentials,
        action: &str,
        mut parameters: BTreeMap<String, String>,
    ) -> CompShareResult<T> {
        parameters.insert("Action".to_string(), action.to_string());
        parameters.insert("PublicKey".to_string(), credentials.public_key.clone());
        let signature = sign_parameters(&credentials.private_key, &parameters);
        parameters.insert("Signature".to_string(), signature);

        let response = self
            .client
            .post(self.base_url.clone())
            .header(
                "U-Timestamp-Ms",
                chrono::Utc::now().timestamp_millis().to_string(),
            )
            .form(&parameters)
            .send()
            .await
            .map_err(map_network_error)?;

        let status = response.status();
        let request_uuid = response
            .headers()
            .get("X-UCloud-Request-UUID")
            .or_else(|| response.headers().get("X-Request-Id"))
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        if let Some(length) = response.content_length() {
            if length as usize > MAX_RESPONSE_BYTES {
                return Err(CompShareError::new(
                    "RESPONSE_TOO_LARGE",
                    "优云智算接口响应超过安全大小限制",
                ));
            }
        }
        let body = response.bytes().await.map_err(map_network_error)?;
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(CompShareError::new(
                "RESPONSE_TOO_LARGE",
                "优云智算接口响应超过安全大小限制",
            ));
        }
        if !status.is_success() {
            return Err(CompShareError {
                code: "HTTP_STATUS_ERROR".to_string(),
                message: format!("优云智算接口 HTTP 状态异常：{}", status.as_u16()),
                ret_code: None,
                request_uuid,
            });
        }

        let value: Value = serde_json::from_slice(&body).map_err(|_| {
            CompShareError::new("INVALID_API_RESPONSE", "优云智算接口返回了无法解析的数据")
        })?;
        let ret_code = parse_ret_code(value.get("RetCode")).unwrap_or(-1);
        if ret_code != 0 {
            let message = value
                .get("Message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let response_request_uuid = value
                .get("RequestUuid")
                .or_else(|| value.get("RequestUUID"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or(request_uuid);
            return Err(CompShareError::api(
                ret_code,
                message,
                response_request_uuid,
            ));
        }

        serde_json::from_value(value).map_err(|_| {
            CompShareError::new(
                "INVALID_API_RESPONSE",
                format!("优云智算接口 {action} 返回的数据结构不完整"),
            )
        })
    }
}

pub struct CompShareProvider {
    api: CompShareApi,
    metadata_path: PathBuf,
    metadata: RwLock<CompShareMetadata>,
}

impl CompShareProvider {
    pub fn new(app_data_dir: PathBuf) -> CompShareResult<Self> {
        Self::new_with_api(app_data_dir, API_BASE_URL)
    }

    fn new_with_api(app_data_dir: PathBuf, api_base_url: &str) -> CompShareResult<Self> {
        fs::create_dir_all(&app_data_dir).map_err(|error| {
            CompShareError::new(
                "METADATA_IO_ERROR",
                format!("无法创建优云智算配置目录：{error}"),
            )
        })?;
        let metadata_path = app_data_dir.join("compshare.json");
        let metadata = if metadata_path.exists() {
            let bytes = fs::read(&metadata_path).map_err(|error| {
                CompShareError::new(
                    "METADATA_IO_ERROR",
                    format!("无法读取优云智算配置：{error}"),
                )
            })?;
            serde_json::from_slice(&bytes)
                .map_err(|_| CompShareError::new("INVALID_METADATA", "优云智算本地配置已损坏"))?
        } else {
            CompShareMetadata::default()
        };
        Ok(Self {
            api: CompShareApi::new(api_base_url)?,
            metadata_path,
            metadata: RwLock::new(metadata),
        })
    }

    pub fn configuration(&self) -> CompShareResult<CompShareConfiguration> {
        let credentials_stored = match credential_entry()?.get_password() {
            Ok(_) => true,
            Err(keyring::Error::NoEntry) => false,
            Err(error) => return Err(keyring_error("读取", error)),
        };
        let metadata = self.read_metadata()?;
        let bound = metadata.bound_instance;
        Ok(CompShareConfiguration {
            credentials_stored,
            bound_instance_id: bound.as_ref().map(|value| value.instance_id.clone()),
            region: bound.as_ref().map(|value| value.region.clone()),
            zone: bound.as_ref().map(|value| value.zone.clone()),
            project_id: bound.and_then(|value| value.project_id),
        })
    }

    pub fn save_credentials(
        &self,
        input: SaveCompShareCredentialsInput,
    ) -> CompShareResult<CompShareConfiguration> {
        let credentials = normalize_credentials(input)?;
        let secret = serde_json::to_string(&credentials).map_err(|_| {
            CompShareError::new("CREDENTIAL_STORE_FAILED", "无法编码优云智算 API 凭据")
        })?;
        credential_entry()?
            .set_password(&secret)
            .map_err(|error| keyring_error("保存", error))?;
        self.configuration()
    }

    pub fn clear_credentials(&self) -> CompShareResult<()> {
        match credential_entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keyring_error("清除", error)),
        }
    }

    pub async fn test_connection(&self) -> CompShareResult<CompShareConnectionTest> {
        let credentials = self.credentials()?;
        let balance = self.get_balance_with(&credentials).await?;
        Ok(CompShareConnectionTest {
            connected: true,
            balance,
        })
    }

    pub async fn get_balance(&self) -> CompShareResult<CompShareBalance> {
        let credentials = self.credentials()?;
        self.get_balance_with(&credentials).await
    }

    async fn get_balance_with(
        &self,
        credentials: &ApiCredentials,
    ) -> CompShareResult<CompShareBalance> {
        let response: BalanceResponseWire = self
            .api
            .invoke(credentials, "GetBalance", BTreeMap::new())
            .await?;
        Ok(response.account_info.into())
    }

    pub async fn list_instances(
        &self,
        input: ListCompShareInstancesInput,
    ) -> CompShareResult<Vec<CompShareInstance>> {
        let credentials = self.credentials()?;
        self.list_instances_with(&credentials, input).await
    }

    async fn list_instances_with(
        &self,
        credentials: &ApiCredentials,
        input: ListCompShareInstancesInput,
    ) -> CompShareResult<Vec<CompShareInstance>> {
        let region = normalize_optional(input.region);
        let zone = normalize_optional(input.zone);
        if zone.is_some() && region.is_none() {
            return Err(CompShareError::new(
                "INVALID_INPUT",
                "按可用区查询实例时必须同时指定地域",
            ));
        }
        let mut offset = 0_u64;
        let mut instances = Vec::new();
        loop {
            let mut parameters = BTreeMap::new();
            parameters.insert("Limit".to_string(), "100".to_string());
            parameters.insert("Offset".to_string(), offset.to_string());
            parameters.insert("WithoutGpu".to_string(), "true".to_string());
            if let Some(value) = region.as_ref() {
                parameters.insert("Region".to_string(), value.clone());
            }
            if let Some(value) = zone.as_ref() {
                parameters.insert("Zone".to_string(), value.clone());
            }
            let response: DescribeInstancesResponseWire = self
                .api
                .invoke(credentials, "DescribeCompShareInstance", parameters)
                .await?;
            let page_len = response.instances.len() as u64;
            instances.extend(response.instances.into_iter().map(|wire| {
                CompShareInstance::from_wire(wire, region.as_deref(), zone.as_deref())
            }));
            offset += page_len;
            if page_len == 0 || offset >= response.total_count || offset >= 10_000 {
                break;
            }
        }
        Ok(instances)
    }

    pub async fn bind_instance(
        &self,
        input: BindCompShareInstanceInput,
    ) -> CompShareResult<CompShareInstance> {
        let instance_id = required_value(input.instance_id, "实例 ID")?;
        let region = required_value(input.region, "地域")?;
        let zone = required_value(input.zone, "可用区")?;
        let credentials = self.credentials()?;
        let instance = self
            .describe_exact(&credentials, &instance_id, &region, &zone)
            .await?;
        let project_id = normalize_optional(input.project_id).or(instance.project_id.clone());
        self.write_metadata(CompShareMetadata {
            bound_instance: Some(BoundInstance {
                instance_id,
                region,
                zone,
                project_id,
            }),
        })?;
        Ok(instance)
    }

    pub async fn bound_instance(&self) -> CompShareResult<CompShareInstance> {
        let credentials = self.credentials()?;
        let bound = self.required_bound_instance()?;
        self.describe_bound(&credentials, &bound).await
    }

    pub async fn start_instance(
        &self,
        mode: CompShareStartMode,
    ) -> CompShareResult<CompShareActionResult> {
        let credentials = self.credentials()?;
        let bound = self.required_bound_instance()?;
        let current = self.describe_bound(&credentials, &bound).await?;
        match current.state {
            CompSharePowerState::Running => {
                let expected = match mode {
                    CompShareStartMode::Gpu => CompShareRunningMode::Gpu,
                    CompShareStartMode::NoGpu => CompShareRunningMode::NoGpu,
                };
                if current.running_mode != expected {
                    return Err(CompShareError::new(
                        "INSTANCE_MODE_CONFLICT",
                        "实例已用另一种算力模式运行，请先明确关机后再启动",
                    ));
                }
                return Ok(CompShareActionResult {
                    request_sent: false,
                    requested_mode: Some(mode),
                    instance: current,
                });
            }
            CompSharePowerState::Stopped => {}
            CompSharePowerState::Starting | CompSharePowerState::Stopping => {
                return Err(CompShareError::new(
                    "INSTANCE_TRANSITIONING",
                    "实例正在切换状态，请稍后重试",
                ));
            }
            CompSharePowerState::Unknown => {
                return Err(CompShareError::new(
                    "UNSUPPORTED_INSTANCE_STATE",
                    format!("无法在状态 {} 下启动实例", current.raw_state),
                ));
            }
        }
        if mode == CompShareStartMode::NoGpu && !current.support_without_gpu_start {
            return Err(CompShareError::new(
                "NO_GPU_NOT_SUPPORTED",
                "此实例不支持无卡启动",
            ));
        }

        let mut parameters = action_parameters(&bound);
        if mode == CompShareStartMode::NoGpu {
            parameters.insert("WithoutGpuSpec".to_string(), "A".to_string());
        }
        let _: InstanceActionResponseWire = self
            .api
            .invoke(&credentials, "StartCompShareInstance", parameters)
            .await?;
        let instance = self.describe_bound(&credentials, &bound).await?;
        Ok(CompShareActionResult {
            request_sent: true,
            requested_mode: Some(mode),
            instance,
        })
    }

    pub async fn stop_instance(&self) -> CompShareResult<CompShareActionResult> {
        let credentials = self.credentials()?;
        let bound = self.required_bound_instance()?;
        let current = self.describe_bound(&credentials, &bound).await?;
        match current.state {
            CompSharePowerState::Stopped => {
                return Ok(CompShareActionResult {
                    request_sent: false,
                    requested_mode: None,
                    instance: current,
                });
            }
            CompSharePowerState::Running => {}
            CompSharePowerState::Starting | CompSharePowerState::Stopping => {
                return Err(CompShareError::new(
                    "INSTANCE_TRANSITIONING",
                    "实例正在切换状态，请稍后重试",
                ));
            }
            CompSharePowerState::Unknown => {
                return Err(CompShareError::new(
                    "UNSUPPORTED_INSTANCE_STATE",
                    format!("无法在状态 {} 下关闭实例", current.raw_state),
                ));
            }
        }
        let parameters = action_parameters(&bound);
        let _: InstanceActionResponseWire = self
            .api
            .invoke(&credentials, "StopCompShareInstance", parameters)
            .await?;
        let instance = self.describe_bound(&credentials, &bound).await?;
        Ok(CompShareActionResult {
            request_sent: true,
            requested_mode: None,
            instance,
        })
    }

    pub async fn update_stop_scheduler(
        &self,
        input: UpdateCompShareStopSchedulerInput,
    ) -> CompShareResult<CompShareSchedulerResult> {
        if input.stop_time <= chrono::Utc::now().timestamp() {
            return Err(CompShareError::new(
                "INVALID_STOP_TIME",
                "定时关机时间必须晚于当前时间",
            ));
        }
        let credentials = self.credentials()?;
        let mut bound = self.required_bound_instance()?;
        let project_id = normalize_optional(input.project_id)
            .or_else(|| bound.project_id.clone())
            .ok_or_else(|| {
                CompShareError::new("PROJECT_ID_REQUIRED", "更新定时关机需要优云智算项目 ID")
            })?;
        let _current = self.describe_bound(&credentials, &bound).await?;
        let mut parameters = action_parameters(&bound);
        parameters.insert("ProjectId".to_string(), project_id.clone());
        parameters.insert("StopTime".to_string(), input.stop_time.to_string());
        let _: EmptyResponseWire = self
            .api
            .invoke(&credentials, "UpdateCompShareStopScheduler", parameters)
            .await?;
        if bound.project_id.as_deref() != Some(project_id.as_str()) {
            bound.project_id = Some(project_id);
            self.write_metadata(CompShareMetadata {
                bound_instance: Some(bound.clone()),
            })?;
        }
        let instance = self.describe_bound(&credentials, &bound).await?;
        Ok(CompShareSchedulerResult {
            stop_time: input.stop_time,
            instance,
        })
    }

    async fn describe_bound(
        &self,
        credentials: &ApiCredentials,
        bound: &BoundInstance,
    ) -> CompShareResult<CompShareInstance> {
        self.describe_exact(credentials, &bound.instance_id, &bound.region, &bound.zone)
            .await
    }

    async fn describe_exact(
        &self,
        credentials: &ApiCredentials,
        instance_id: &str,
        region: &str,
        zone: &str,
    ) -> CompShareResult<CompShareInstance> {
        let mut parameters = BTreeMap::new();
        parameters.insert("Limit".to_string(), "2".to_string());
        parameters.insert("Offset".to_string(), "0".to_string());
        parameters.insert("Region".to_string(), region.to_string());
        parameters.insert("Zone".to_string(), zone.to_string());
        parameters.insert("UHostIds.0".to_string(), instance_id.to_string());
        parameters.insert("WithoutGpu".to_string(), "true".to_string());
        let response: DescribeInstancesResponseWire = self
            .api
            .invoke(credentials, "DescribeCompShareInstance", parameters)
            .await?;
        let mut matches = response
            .instances
            .into_iter()
            .map(|wire| CompShareInstance::from_wire(wire, Some(region), Some(zone)))
            .filter(|instance| instance.instance_id == instance_id)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(CompShareError::new(
                "INSTANCE_NOT_FOUND",
                "在指定地域和可用区中找不到绑定的实例",
            ));
        }
        let instance = matches.remove(0);
        if instance.region != region || instance.zone != zone {
            return Err(CompShareError::new(
                "INSTANCE_LOCATION_MISMATCH",
                "实例返回的地域或可用区与绑定信息不一致",
            ));
        }
        Ok(instance)
    }

    fn credentials(&self) -> CompShareResult<ApiCredentials> {
        let secret = match credential_entry()?.get_password() {
            Ok(value) => value,
            Err(keyring::Error::NoEntry) => {
                return Err(CompShareError::new(
                    "CREDENTIALS_NOT_CONFIGURED",
                    "尚未保存优云智算 API 凭据",
                ));
            }
            Err(error) => return Err(keyring_error("读取", error)),
        };
        serde_json::from_str(&secret).map_err(|_| {
            CompShareError::new(
                "INVALID_STORED_CREDENTIALS",
                "Windows 凭据管理器中的优云智算凭据格式无效",
            )
        })
    }

    fn required_bound_instance(&self) -> CompShareResult<BoundInstance> {
        self.read_metadata()?
            .bound_instance
            .ok_or_else(|| CompShareError::new("INSTANCE_NOT_BOUND", "尚未绑定优云智算实例"))
    }

    fn read_metadata(&self) -> CompShareResult<CompShareMetadata> {
        self.metadata
            .read()
            .map(|value| value.clone())
            .map_err(|_| CompShareError::new("METADATA_LOCK_ERROR", "优云智算配置锁已损坏"))
    }

    fn write_metadata(&self, metadata: CompShareMetadata) -> CompShareResult<()> {
        let bytes = serde_json::to_vec_pretty(&metadata)
            .map_err(|_| CompShareError::new("METADATA_IO_ERROR", "无法编码优云智算配置"))?;
        let temporary_path = self.metadata_path.with_extension("json.tmp");
        fs::write(&temporary_path, bytes).map_err(|error| {
            CompShareError::new(
                "METADATA_IO_ERROR",
                format!("无法写入优云智算配置：{error}"),
            )
        })?;
        fs::rename(&temporary_path, &self.metadata_path).map_err(|error| {
            CompShareError::new(
                "METADATA_IO_ERROR",
                format!("无法保存优云智算配置：{error}"),
            )
        })?;
        let mut guard = self
            .metadata
            .write()
            .map_err(|_| CompShareError::new("METADATA_LOCK_ERROR", "优云智算配置锁已损坏"))?;
        *guard = metadata;
        Ok(())
    }
}

fn credential_entry() -> CompShareResult<Entry> {
    Entry::new(CREDENTIAL_SERVICE, CREDENTIAL_USER).map_err(|error| keyring_error("访问", error))
}

fn keyring_error(operation: &str, error: keyring::Error) -> CompShareError {
    CompShareError::new(
        "CREDENTIAL_STORE_ERROR",
        format!("无法{operation} Windows 凭据管理器：{error}"),
    )
}

fn normalize_credentials(input: SaveCompShareCredentialsInput) -> CompShareResult<ApiCredentials> {
    let public_key = required_value(input.public_key, "公钥")?;
    let private_key = required_value(input.private_key, "私钥")?;
    if public_key.len() > 4096 || private_key.len() > 4096 {
        return Err(CompShareError::new(
            "INVALID_CREDENTIALS",
            "优云智算 API 凭据长度异常",
        ));
    }
    Ok(ApiCredentials {
        public_key,
        private_key,
    })
}

fn required_value(value: String, label: &str) -> CompShareResult<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(CompShareError::new(
            "INVALID_INPUT",
            format!("{label}不能为空"),
        ));
    }
    Ok(value)
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    })
}

fn action_parameters(bound: &BoundInstance) -> BTreeMap<String, String> {
    let mut parameters = BTreeMap::new();
    parameters.insert("Region".to_string(), bound.region.clone());
    parameters.insert("Zone".to_string(), bound.zone.clone());
    parameters.insert("UHostId".to_string(), bound.instance_id.clone());
    if let Some(project_id) = bound.project_id.as_ref() {
        parameters.insert("ProjectId".to_string(), project_id.clone());
    }
    parameters
}

fn sign_parameters(private_key: &str, parameters: &BTreeMap<String, String>) -> String {
    let mut simplified = String::new();
    for (key, value) in parameters {
        simplified.push_str(key);
        simplified.push_str(value);
    }
    simplified.push_str(private_key);
    format!("{:x}", Sha1::digest(simplified.as_bytes()))
}

fn map_network_error(error: reqwest::Error) -> CompShareError {
    if error.is_timeout() {
        CompShareError::new("REQUEST_TIMEOUT", "连接优云智算接口超时")
    } else if error.is_connect() {
        CompShareError::new("CONNECTION_FAILED", "无法连接优云智算接口")
    } else {
        CompShareError::new("NETWORK_ERROR", format!("优云智算网络请求失败：{error}"))
    }
}

fn parse_ret_code(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(value)) => value.as_i64(),
        Some(Value::String(value)) => value.parse().ok(),
        _ => None,
    }
}

fn parse_power_state(raw: &str) -> CompSharePowerState {
    match raw.to_ascii_lowercase().as_str() {
        "running" => CompSharePowerState::Running,
        "stopped" => CompSharePowerState::Stopped,
        "starting" | "initializing" | "pending" => CompSharePowerState::Starting,
        "stopping" => CompSharePowerState::Stopping,
        _ => CompSharePowerState::Unknown,
    }
}

impl CompShareInstance {
    fn from_wire(wire: CompShareInstanceWire, region: Option<&str>, zone: Option<&str>) -> Self {
        let raw_state = wire.state.unwrap_or_else(|| "Unknown".to_string());
        let state = parse_power_state(&raw_state);
        let running_mode = match state {
            CompSharePowerState::Running if wire.gpu.unwrap_or_default() == 0 => {
                CompShareRunningMode::NoGpu
            }
            CompSharePowerState::Running => CompShareRunningMode::Gpu,
            CompSharePowerState::Stopped => CompShareRunningMode::Stopped,
            CompSharePowerState::Starting | CompSharePowerState::Stopping => {
                CompShareRunningMode::Transitioning
            }
            CompSharePowerState::Unknown => CompShareRunningMode::Unknown,
        };
        Self {
            instance_id: wire.instance_id.unwrap_or_default(),
            name: wire.name,
            region: wire
                .region
                .or_else(|| region.map(str::to_owned))
                .unwrap_or_default(),
            zone: wire
                .zone
                .or_else(|| zone.map(str::to_owned))
                .unwrap_or_default(),
            state,
            raw_state,
            running_mode,
            cpu: wire.cpu,
            memory_mb: wire.memory,
            gpu_count: wire.gpu,
            gpu_type: wire.gpu_type,
            support_without_gpu_start: wire.support_without_gpu_start.unwrap_or(false),
            ssh_login_command: wire.ssh_login_command,
            start_time: wire.start_time,
            stop_time: wire.stop_time,
            release_time: wire.release_time,
            stop_scheduler_time: wire.stop_scheduler_time.or(wire.scheduler_stop_time),
            instance_price: wire.instance_price,
            project_id: wire.project_id,
        }
    }
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
struct ApiResponseBase {
    #[serde(rename = "RetCode")]
    _ret_code: Value,
}

#[derive(Deserialize)]
struct BalanceResponseWire {
    #[serde(rename = "RetCode")]
    _ret_code: Value,
    #[serde(rename = "AccountInfo")]
    account_info: AccountInfoWire,
}

#[derive(Deserialize)]
struct AccountInfoWire {
    #[serde(rename = "Amount")]
    amount: Option<String>,
    #[serde(rename = "AmountAvailable")]
    amount_available: Option<String>,
    #[serde(rename = "AmountCredit")]
    amount_credit: Option<String>,
    #[serde(rename = "AmountFree")]
    amount_free: Option<String>,
    #[serde(rename = "AmountFreeze")]
    amount_freeze: Option<String>,
}

impl From<AccountInfoWire> for CompShareBalance {
    fn from(value: AccountInfoWire) -> Self {
        Self {
            amount: value.amount,
            amount_available: value.amount_available,
            amount_credit: value.amount_credit,
            amount_free: value.amount_free,
            amount_freeze: value.amount_freeze,
        }
    }
}

#[derive(Deserialize)]
struct DescribeInstancesResponseWire {
    #[serde(rename = "RetCode")]
    _ret_code: Value,
    #[serde(rename = "TotalCount", default)]
    total_count: u64,
    #[serde(rename = "UHostSet", default)]
    instances: Vec<CompShareInstanceWire>,
}

#[derive(Deserialize)]
struct CompShareInstanceWire {
    #[serde(rename = "UHostId")]
    instance_id: Option<String>,
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Region")]
    region: Option<String>,
    #[serde(rename = "Zone")]
    zone: Option<String>,
    #[serde(rename = "ProjectId")]
    project_id: Option<String>,
    #[serde(rename = "State")]
    state: Option<String>,
    #[serde(rename = "CPU")]
    cpu: Option<u32>,
    #[serde(rename = "Memory")]
    memory: Option<u64>,
    #[serde(rename = "GPU")]
    gpu: Option<u32>,
    #[serde(rename = "GpuType")]
    gpu_type: Option<String>,
    #[serde(rename = "SupportWithoutGpuStart")]
    support_without_gpu_start: Option<bool>,
    #[serde(rename = "SshLoginCommand")]
    ssh_login_command: Option<String>,
    #[serde(rename = "StartTime")]
    start_time: Option<i64>,
    #[serde(rename = "StopTime")]
    stop_time: Option<i64>,
    #[serde(rename = "ReleaseTime")]
    release_time: Option<i64>,
    #[serde(rename = "StopSchedulerTime")]
    stop_scheduler_time: Option<i64>,
    #[serde(rename = "SchedulerStopTime")]
    scheduler_stop_time: Option<i64>,
    #[serde(rename = "InstancePrice")]
    instance_price: Option<f64>,
}

#[derive(Deserialize)]
struct InstanceActionResponseWire {
    #[serde(rename = "RetCode")]
    _ret_code: Value,
    #[serde(rename = "UHostId")]
    _instance_id: String,
}

#[derive(Deserialize)]
struct EmptyResponseWire {
    #[serde(rename = "RetCode")]
    _ret_code: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    fn fake_credentials() -> ApiCredentials {
        ApiCredentials {
            public_key: "fake-public-key".to_string(),
            private_key: "fake-private-key".to_string(),
        }
    }

    fn mock_server(response_body: &'static str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let address = listener.local_addr().expect("local address");
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = stream.read(&mut buffer).expect("read request");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                let header_end = request
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|position| position + 4);
                if let Some(header_end) = header_end {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length: ")
                                .or_else(|| line.strip_prefix("Content-Length: "))
                        })
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or_default();
                    if request.len() >= header_end + content_length {
                        break;
                    }
                }
            }
            let _ = sender.send(String::from_utf8_lossy(&request).into_owned());
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        });
        (format!("http://{address}"), receiver)
    }

    #[test]
    fn signature_matches_ucloud_reference_algorithm() {
        let mut parameters = BTreeMap::new();
        parameters.insert("foo".to_string(), "bar".to_string());
        assert_eq!(
            sign_parameters("my_private_key", &parameters),
            "634edc1bb957c0d65e5ab5494cf3b7784fbc87af"
        );
    }

    #[test]
    fn state_mapping_distinguishes_gpu_and_no_gpu() {
        let no_gpu = CompShareInstance::from_wire(
            CompShareInstanceWire {
                instance_id: Some("uhost-test".to_string()),
                name: None,
                region: Some("cn-test".to_string()),
                zone: Some("cn-test-01".to_string()),
                project_id: None,
                state: Some("Running".to_string()),
                cpu: Some(2),
                memory: Some(4096),
                gpu: Some(0),
                gpu_type: Some("5090".to_string()),
                support_without_gpu_start: Some(true),
                ssh_login_command: None,
                start_time: None,
                stop_time: None,
                release_time: None,
                stop_scheduler_time: None,
                scheduler_stop_time: None,
                instance_price: None,
            },
            None,
            None,
        );
        assert_eq!(no_gpu.state, CompSharePowerState::Running);
        assert_eq!(no_gpu.running_mode, CompShareRunningMode::NoGpu);
        assert_eq!(parse_power_state("Stopping"), CompSharePowerState::Stopping);
        assert_eq!(parse_power_state("surprise"), CompSharePowerState::Unknown);
    }

    #[tokio::test]
    async fn signed_form_request_and_balance_response_are_compatible() {
        let body = r#"{"RetCode":0,"AccountInfo":{"Amount":"13.80","AmountAvailable":"12.50","AmountCredit":"0","AmountFree":"1.30","AmountFreeze":"0"}}"#;
        let (url, captured) = mock_server(body);
        let api = CompShareApi::new(&url).expect("api client");
        let response: BalanceResponseWire = api
            .invoke(&fake_credentials(), "GetBalance", BTreeMap::new())
            .await
            .expect("valid response");
        let balance: CompShareBalance = response.account_info.into();
        assert_eq!(balance.amount_available.as_deref(), Some("12.50"));

        let request = captured.recv().expect("captured request");
        assert!(request.starts_with("POST / HTTP/1.1"));
        assert!(request.contains("Action=GetBalance"));
        assert!(request.contains("PublicKey=fake-public-key"));
        assert!(request.contains("Signature="));
        assert!(!request.contains("fake-private-key"));
    }

    #[tokio::test]
    async fn api_errors_keep_ret_code_and_request_id() {
        let body = r#"{"RetCode":12345,"Message":"fake error","RequestUuid":"request-test"}"#;
        let (url, _) = mock_server(body);
        let api = CompShareApi::new(&url).expect("api client");
        let error = api
            .invoke::<ApiResponseBase>(&fake_credentials(), "GetBalance", BTreeMap::new())
            .await
            .expect_err("API should fail");
        assert_eq!(error.code, "COMPSHARE_API_ERROR");
        assert_eq!(error.ret_code, Some(12345));
        assert_eq!(error.request_uuid.as_deref(), Some("request-test"));
    }
}
