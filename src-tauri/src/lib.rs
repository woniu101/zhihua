mod asset;
mod audio_inspector;
mod comp_share;
mod compute_control;
mod compute_pool;
mod export;
mod frame_composition;
mod frame_profile;
mod generation;
mod job_queue;
#[cfg(test)]
mod live_validation;
mod llm;
mod service;
mod source;
mod ssh_tunnel;
mod storage;
mod storyboard;
mod tts;

use asset::{
    AssetError, AssetItem, AssetStorage, ImportAssetFilesInput, ImportAssetPayloadInput,
    ReplaceAssetFileInput, SetCurrentAssetVersionInput, UnlinkAssetInput, UpdateAssetInput,
};
use audio_inspector::{CandidateAudioInspection, InspectCandidateAudioInput};
use comp_share::{
    BindCompShareInstanceInput, CompShareActionResult, CompShareBalance, CompShareConfiguration,
    CompShareConnectionTest, CompShareCreatePreflight, CompShareCreateSpec, CompShareError,
    CompShareInstance, CompShareInstanceLocator, CompSharePowerState, CompShareProvider,
    CompShareRunningMode, CompShareSchedulerResult, CompShareStartMode, CompShareZone,
    ListCompShareInstancesInput, SaveCompShareCredentialsInput, UpdateCompShareStopSchedulerInput,
};
use compute_control::{
    ComputeCleanupPolicy, ComputeControlError, ComputeControlStore, ComputeInstanceRole,
    ComputeOperation, ComputeOperationAction, ComputeOperationStatus, ComputeServiceState,
    ComputeWorkerLease, ComputeWorkerReadiness, ManagedComputeInstance, ReleaseEligibility,
};
use compute_pool::{plan_compute_pool, ComputePoolPlan, ComputePoolPlanInput};
use export::{
    ExportCapability, ExportError, ExportProjectInput, FfmpegExporter, PreviewProjectInput,
    ProjectExport,
};
use frame_composition::{
    FrameComposition, FrameCompositionError, FrameCompositionStorage, GetFrameCompositionInput,
    PrepareFrameDerivativeInput, SaveFrameCompositionInput,
};
use frame_profile::{FrameAspectRatio, FrameProfile};
use generation::{
    CandidateVersion, EnhancedVersion, GenerationError, GenerationStorage, RecordCandidateInput,
    RecordEnhancedInput,
};
use job_queue::{JobQueueError, JobQueueStorage, LocalJob};
use llm::{
    AnalyzeSourcesInput, CreateStoryboardInput, KnowledgePoint, LlmConfiguration,
    LlmConnectionTest, LlmError, LlmProvider, LlmSource, ReviseSceneInput,
    SaveLlmConfigurationInput, SceneRevisionProposal,
};
use service::{
    DownloadServiceArtifactInput, SaveServiceConnectionInput, ServiceArtifactDownload,
    ServiceClient, ServiceConnectionError, ServiceConnectionInfo, ServiceConnectionResult,
    ServiceInputUpload, ServiceJob, ServiceProbe, SubmitServiceJobInput,
};
use source::{
    CreatePastedSourceInput, ImportSourceFileInput, SetSourceEnabledInput, Source, SourceStorage,
};
use ssh_tunnel::{SaveTunnelConfigurationInput, SshTunnelManager, TunnelError, TunnelStatus};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};
use storage::{
    CreateProjectInput, Project, ProjectStatus, ProjectStorage, StorageInfo, UpdateProjectInput,
};
use storyboard::{
    CandidateQuality, GenerationMode, NarrationMode, ReorderScenesInput, SceneDraft, SceneStatus,
    SourceReference, StoryboardStorage,
};
use tauri::{AppHandle, Manager, State};
use tts::{
    ImportNarrationInput, NarrationArtifact, SynthesizeNarrationInput, SystemTtsProvider,
    SystemVoice, TtsError,
};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadCompletedJobInput {
    project_id: String,
    job_id: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadCompletedUpscaleInput {
    project_id: String,
    job_id: String,
    source_candidate_id: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CandidateVersionsInput {
    project_id: String,
    scene_id: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelectCandidateVersionInput {
    project_id: String,
    scene_id: String,
    version_id: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateManagedComputeInstanceInput {
    idempotency_key: String,
    name: String,
    spec: CompShareCreateSpec,
    role: ComputeInstanceRole,
    confirmed: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseManagedComputeInstanceInput {
    idempotency_key: String,
    instance_id: String,
    #[serde(default)]
    release_data_disk: bool,
    confirmed: bool,
}

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum ComputeKeepAlivePolicy {
    Economy,
    Availability,
    Continuous,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ComputePolicySnapshot {
    policy: ComputeKeepAlivePolicy,
    idle_shutdown_minutes: Option<u64>,
    hard_limit_minutes: u64,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredComputePolicy {
    policy: ComputeKeepAlivePolicy,
}

#[derive(Clone)]
struct ComputeLifecycle {
    revision: Arc<AtomicU64>,
    policy: Arc<RwLock<ComputeKeepAlivePolicy>>,
    policy_path: Arc<PathBuf>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ApplicationExitProtection {
    active_remote_tasks: u64,
    action: String,
    detail: String,
}

impl ComputeLifecycle {
    fn load(policy_path: PathBuf) -> Self {
        let policy = fs::read(&policy_path)
            .ok()
            .and_then(|content| serde_json::from_slice::<StoredComputePolicy>(&content).ok())
            .map(|stored| stored.policy)
            .unwrap_or(ComputeKeepAlivePolicy::Economy);
        Self {
            revision: Arc::new(AtomicU64::new(0)),
            policy: Arc::new(RwLock::new(policy)),
            policy_path: Arc::new(policy_path),
        }
    }

    fn invalidate_idle_shutdown(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn is_current(&self, revision: u64) -> bool {
        self.revision.load(Ordering::SeqCst) == revision
    }

    fn policy(&self) -> ComputeKeepAlivePolicy {
        *self
            .policy
            .read()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn snapshot(&self) -> ComputePolicySnapshot {
        let policy = self.policy();
        ComputePolicySnapshot {
            policy,
            idle_shutdown_minutes: match policy {
                ComputeKeepAlivePolicy::Economy => Some(3),
                ComputeKeepAlivePolicy::Availability => Some(15),
                ComputeKeepAlivePolicy::Continuous => None,
            },
            hard_limit_minutes: match policy {
                ComputeKeepAlivePolicy::Economy => 60,
                ComputeKeepAlivePolicy::Availability => 180,
                ComputeKeepAlivePolicy::Continuous => 720,
            },
        }
    }

    fn set_policy(&self, policy: ComputeKeepAlivePolicy) -> Result<ComputePolicySnapshot, String> {
        if let Some(parent) = self.policy_path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("无法创建算力策略目录：{error}"))?;
        }
        let content = serde_json::to_vec_pretty(&StoredComputePolicy { policy })
            .map_err(|error| format!("无法保存算力策略：{error}"))?;
        fs::write(self.policy_path.as_ref(), content)
            .map_err(|error| format!("无法写入算力策略：{error}"))?;
        *self
            .policy
            .write()
            .unwrap_or_else(|error| error.into_inner()) = policy;
        self.invalidate_idle_shutdown();
        Ok(self.snapshot())
    }
}

#[tauri::command]
fn get_compute_policy(lifecycle: State<'_, ComputeLifecycle>) -> ComputePolicySnapshot {
    lifecycle.snapshot()
}

#[tauri::command]
async fn set_compute_policy(
    app: AppHandle,
    lifecycle: State<'_, ComputeLifecycle>,
    provider: State<'_, CompShareProvider>,
    policy: ComputeKeepAlivePolicy,
) -> Result<ComputePolicySnapshot, String> {
    let previous = lifecycle.policy();
    let snapshot = lifecycle.set_policy(policy)?;
    match provider.bound_instance().await {
        Ok(instance)
            if instance.state == CompSharePowerState::Running
                && instance.running_mode == CompShareRunningMode::Gpu =>
        {
            if let Err(error) = provider
                .update_stop_scheduler(UpdateCompShareStopSchedulerInput {
                    stop_time: chrono::Utc::now().timestamp()
                        + (snapshot.hard_limit_minutes as i64) * 60,
                    project_id: instance.project_id,
                })
                .await
            {
                let _ = lifecycle.set_policy(previous);
                return Err(format!(
                    "算力策略未切换：无法同步更新平台关机保障（{}）",
                    error.message
                ));
            }
        }
        _ => {}
    }
    restart_idle_gpu_shutdown(&app, &lifecycle);
    Ok(snapshot)
}

#[tauri::command]
fn get_compshare_configuration(
    provider: State<'_, CompShareProvider>,
) -> Result<CompShareConfiguration, CompShareError> {
    provider.configuration()
}

#[tauri::command]
fn save_compshare_credentials(
    provider: State<'_, CompShareProvider>,
    input: SaveCompShareCredentialsInput,
) -> Result<CompShareConfiguration, CompShareError> {
    provider.save_credentials(input)
}

#[tauri::command]
fn clear_compshare_credentials(
    provider: State<'_, CompShareProvider>,
) -> Result<(), CompShareError> {
    provider.clear_credentials()
}

#[tauri::command]
async fn test_compshare_connection(
    provider: State<'_, CompShareProvider>,
) -> Result<CompShareConnectionTest, CompShareError> {
    provider.test_connection().await
}

#[tauri::command]
async fn get_compshare_balance(
    provider: State<'_, CompShareProvider>,
) -> Result<CompShareBalance, CompShareError> {
    provider.get_balance().await
}

#[tauri::command]
async fn list_compshare_instances(
    provider: State<'_, CompShareProvider>,
    input: ListCompShareInstancesInput,
) -> Result<Vec<CompShareInstance>, CompShareError> {
    provider.list_instances(input).await
}

#[tauri::command]
async fn list_compshare_zones(
    provider: State<'_, CompShareProvider>,
) -> Result<Vec<CompShareZone>, CompShareError> {
    provider.list_zones().await
}

#[tauri::command]
async fn preflight_compshare_create(
    provider: State<'_, CompShareProvider>,
    spec: CompShareCreateSpec,
) -> Result<CompShareCreatePreflight, CompShareError> {
    provider.preflight_create(spec).await
}

#[tauri::command]
fn list_managed_compute_instances(
    store: State<'_, ComputeControlStore>,
) -> Result<Vec<ManagedComputeInstance>, String> {
    store.list_instances().map_err(|error| error.message)
}

#[tauri::command]
fn set_user_compute_worker_enabled(
    store: State<'_, ComputeControlStore>,
    instance_id: String,
    enabled: bool,
) -> Result<ManagedComputeInstance, String> {
    store
        .set_user_instance_worker_enabled(&instance_id, enabled)
        .map_err(|error| error.message)
}

#[tauri::command]
fn list_compute_worker_readiness(
    store: State<'_, ComputeControlStore>,
) -> Result<Vec<ComputeWorkerReadiness>, String> {
    store.list_worker_readiness().map_err(|error| error.message)
}

#[tauri::command]
async fn reconcile_compute_instances(
    provider: State<'_, CompShareProvider>,
    store: State<'_, ComputeControlStore>,
) -> Result<Vec<ManagedComputeInstance>, String> {
    let platform_instances = provider
        .list_instances(ListCompShareInstancesInput {
            region: None,
            zone: None,
        })
        .await
        .map_err(|error| error.message)?;
    let configuration = provider.configuration().map_err(|error| error.message)?;
    store
        .reconcile(
            &platform_instances,
            configuration.bound_instance_id.as_deref(),
        )
        .map_err(|error| error.message)
}

#[tauri::command]
fn get_compute_release_eligibility(
    store: State<'_, ComputeControlStore>,
    queue: State<'_, JobQueueStorage>,
    instance_id: String,
) -> Result<ReleaseEligibility, String> {
    compute_release_eligibility(&store, &queue, &instance_id, None)
}

fn compute_release_eligibility(
    store: &ComputeControlStore,
    queue: &JobQueueStorage,
    instance_id: &str,
    excluded_idempotency_key: Option<&str>,
) -> Result<ReleaseEligibility, String> {
    let mut eligibility = match excluded_idempotency_key {
        Some(key) => store.release_eligibility_excluding(instance_id, Some(key)),
        None => store.release_eligibility(instance_id),
    }
    .map_err(|error| error.message)?;
    let worker_id = format!("instance:{instance_id}:gpu:0");
    if queue
        .has_unsettled_remote_jobs_for_worker(&worker_id)
        .map_err(|error| error.message)?
    {
        eligibility
            .reasons
            .push("实例仍有尚未取回并校验的远端结果".to_owned());
        eligibility.allowed = false;
    }
    Ok(eligibility)
}

fn uncertain_compshare_result(error: &CompShareError) -> bool {
    matches!(
        error.code.as_str(),
        "REQUEST_TIMEOUT"
            | "CONNECTION_FAILED"
            | "NETWORK_ERROR"
            | "HTTP_STATUS_ERROR"
            | "INVALID_API_RESPONSE"
            | "RESPONSE_TOO_LARGE"
    )
}

#[tauri::command]
async fn create_managed_compshare_instance(
    provider: State<'_, CompShareProvider>,
    store: State<'_, ComputeControlStore>,
    input: CreateManagedComputeInstanceInput,
) -> Result<ComputeOperation, String> {
    if !input.confirmed {
        return Err("创建实例前必须向用户展示地域、规格、镜像和价格并获得确认".to_owned());
    }
    if !matches!(
        input.role,
        ComputeInstanceRole::Elastic | ComputeInstanceRole::Test
    ) {
        return Err("客户端只能自动创建弹性实例或测试实例".to_owned());
    }
    let payload = serde_json::json!({
        "name": input.name,
        "role": input.role,
        "spec": input.spec,
    });
    let (operation, claimed) = store
        .claim_operation(
            &input.idempotency_key,
            None,
            ComputeOperationAction::Create,
            &payload,
        )
        .map_err(|error| error.message)?;
    if !claimed {
        if operation.action != ComputeOperationAction::Create || operation.payload != payload {
            return Err("该幂等键已用于另一项算力操作".to_owned());
        }
        return Ok(operation);
    }

    let preflight = match provider.preflight_create(input.spec.clone()).await {
        Ok(result) if result.capacity_available => result,
        Ok(_) => {
            let operation = store
                .finish_operation(
                    &input.idempotency_key,
                    ComputeOperationStatus::Failed,
                    None,
                    None,
                    Some("所选地域当前没有匹配的空闲实例"),
                )
                .map_err(|error| error.message)?;
            return Ok(operation);
        }
        Err(error) => {
            let status = if uncertain_compshare_result(&error) {
                ComputeOperationStatus::Unknown
            } else {
                ComputeOperationStatus::Failed
            };
            let operation = store
                .finish_operation(
                    &input.idempotency_key,
                    status,
                    None,
                    error.request_uuid.as_deref(),
                    Some(&error.message),
                )
                .map_err(|store_error| store_error.message)?;
            return Ok(operation);
        }
    };
    debug_assert!(preflight.capacity_available);

    let created = match provider
        .create_instance(input.spec.clone(), input.name.clone())
        .await
    {
        Ok(result) => result,
        Err(error) => {
            let status = if uncertain_compshare_result(&error) {
                ComputeOperationStatus::Unknown
            } else {
                ComputeOperationStatus::Failed
            };
            return store
                .finish_operation(
                    &input.idempotency_key,
                    status,
                    None,
                    error.request_uuid.as_deref(),
                    Some(&error.message),
                )
                .map_err(|store_error| store_error.message);
        }
    };
    let instance_id = created.instance_ids[0].clone();
    let placeholder = CompShareInstance {
        instance_id: instance_id.clone(),
        name: Some(input.name),
        region: input.spec.region.clone(),
        zone: input.spec.zone.clone(),
        state: CompSharePowerState::Starting,
        raw_state: "Creating".to_owned(),
        running_mode: CompShareRunningMode::Transitioning,
        cpu: Some(input.spec.cpu),
        memory_mb: Some(input.spec.memory_mb),
        gpu_count: Some(input.spec.gpu_count),
        gpu_type: Some(input.spec.gpu_type.clone()),
        support_without_gpu_start: false,
        ssh_login_command: None,
        start_time: None,
        stop_time: None,
        release_time: None,
        stop_scheduler_time: None,
        instance_price: preflight.estimated_hourly_price,
        disk_price: None,
        image_price: None,
        image_id: Some(input.spec.image_id.clone()),
        charge_type: Some(input.spec.charge_type.clone()),
        project_id: input.spec.project_id.clone(),
    };
    store
        .register_zhihua_instance(&placeholder, input.role)
        .map_err(|error| error.message)?;

    if let Ok(instance) = provider
        .describe_instance(CompShareInstanceLocator {
            instance_id: instance_id.clone(),
            region: input.spec.region,
            zone: input.spec.zone,
            project_id: input.spec.project_id,
        })
        .await
    {
        store
            .register_zhihua_instance(&instance, input.role)
            .map_err(|error| error.message)?;
    }
    store
        .finish_operation(
            &input.idempotency_key,
            ComputeOperationStatus::Succeeded,
            Some(&instance_id),
            created.request_uuid.as_deref(),
            None,
        )
        .map_err(|error| error.message)
}

#[tauri::command]
async fn release_managed_compshare_instance(
    provider: State<'_, CompShareProvider>,
    store: State<'_, ComputeControlStore>,
    queue: State<'_, JobQueueStorage>,
    input: ReleaseManagedComputeInstanceInput,
) -> Result<ComputeOperation, String> {
    if !input.confirmed {
        return Err("释放实例需要用户明确确认".to_owned());
    }
    release_managed_compshare_instance_impl(&provider, &store, &queue, input).await
}

async fn release_managed_compshare_instance_impl(
    provider: &CompShareProvider,
    store: &ComputeControlStore,
    queue: &JobQueueStorage,
    input: ReleaseManagedComputeInstanceInput,
) -> Result<ComputeOperation, String> {
    let payload = serde_json::json!({
        "instanceId": input.instance_id,
        "releaseDataDisk": input.release_data_disk,
    });
    let (operation, claimed) = store
        .claim_operation(
            &input.idempotency_key,
            Some(&input.instance_id),
            ComputeOperationAction::Terminate,
            &payload,
        )
        .map_err(|error| error.message)?;
    if !claimed {
        if operation.action != ComputeOperationAction::Terminate || operation.payload != payload {
            return Err("该幂等键已用于另一项算力操作".to_owned());
        }
        return Ok(operation);
    }

    let eligibility = compute_release_eligibility(
        store,
        queue,
        &input.instance_id,
        Some(&input.idempotency_key),
    )?;
    if !eligibility.allowed {
        return store
            .finish_operation(
                &input.idempotency_key,
                ComputeOperationStatus::Failed,
                Some(&input.instance_id),
                None,
                Some(&eligibility.reasons.join("；")),
            )
            .map_err(|error| error.message);
    }
    let instance = store
        .get_instance(&input.instance_id)
        .map_err(|error| error.message)?;
    store
        .mark_terminating(&input.instance_id)
        .map_err(|error| error.message)?;
    let result = provider
        .terminate_instance(
            CompShareInstanceLocator {
                instance_id: instance.instance_id.clone(),
                region: instance.region,
                zone: instance.zone,
                project_id: instance.project_id,
            },
            input.release_data_disk,
        )
        .await;
    match result {
        Ok(result) => {
            store
                .mark_terminated(&input.instance_id)
                .map_err(|error| error.message)?;
            store
                .finish_operation(
                    &input.idempotency_key,
                    ComputeOperationStatus::Succeeded,
                    Some(&input.instance_id),
                    result.request_uuid.as_deref(),
                    None,
                )
                .map_err(|error| error.message)
        }
        Err(error) => {
            let status = if uncertain_compshare_result(&error) {
                ComputeOperationStatus::Unknown
            } else {
                ComputeOperationStatus::Failed
            };
            store
                .finish_operation(
                    &input.idempotency_key,
                    status,
                    Some(&input.instance_id),
                    error.request_uuid.as_deref(),
                    Some(&error.message),
                )
                .map_err(|store_error| store_error.message)
        }
    }
}

async fn release_idle_elastic_instance(
    provider: &CompShareProvider,
    store: &ComputeControlStore,
    queue: &JobQueueStorage,
    instance: &ManagedComputeInstance,
) -> Result<Option<ComputeOperation>, String> {
    if instance.role != ComputeInstanceRole::Elastic
        || instance.cleanup_policy != ComputeCleanupPolicy::ReleaseWhenIdle
        || !instance.platform_state.eq_ignore_ascii_case("stopped")
        || instance.running_mode != "stopped"
    {
        return Ok(None);
    }
    let eligibility = compute_release_eligibility(store, queue, &instance.instance_id, None)?;
    if !eligibility.allowed {
        return Ok(None);
    }
    let stop_marker = instance.stop_time.unwrap_or(0);
    let operation = release_managed_compshare_instance_impl(
        provider,
        store,
        queue,
        ReleaseManagedComputeInstanceInput {
            idempotency_key: format!("auto-release:{}:{stop_marker}", instance.instance_id),
            instance_id: instance.instance_id.clone(),
            release_data_disk: false,
            confirmed: true,
        },
    )
    .await?;
    Ok(Some(operation))
}

#[tauri::command]
async fn bind_compshare_instance(
    provider: State<'_, CompShareProvider>,
    store: State<'_, ComputeControlStore>,
    input: BindCompShareInstanceInput,
) -> Result<CompShareInstance, String> {
    let instance = provider
        .bind_instance(input)
        .await
        .map_err(|error| error.message)?;
    store
        .adopt_primary(&instance)
        .map_err(|error| error.message)?;
    Ok(instance)
}

#[tauri::command]
async fn get_bound_compshare_instance(
    provider: State<'_, CompShareProvider>,
) -> Result<CompShareInstance, CompShareError> {
    provider.bound_instance().await
}

#[tauri::command]
async fn start_compshare_instance(
    provider: State<'_, CompShareProvider>,
    lifecycle: State<'_, ComputeLifecycle>,
    mode: CompShareStartMode,
) -> Result<CompShareActionResult, String> {
    let initial = provider
        .start_instance(mode)
        .await
        .map_err(|error| error.message)?;
    if mode != CompShareStartMode::Gpu {
        return Ok(initial);
    }
    let hard_limit_minutes = lifecycle.snapshot().hard_limit_minutes;
    if let Err(error) = provider
        .update_stop_scheduler(UpdateCompShareStopSchedulerInput {
            stop_time: chrono::Utc::now().timestamp() + (hard_limit_minutes as i64) * 60,
            project_id: initial.instance.project_id.clone(),
        })
        .await
    {
        let _ = wait_for_instance_state(
            &provider,
            CompSharePowerState::Running,
            Some(CompShareRunningMode::Gpu),
            Duration::from_secs(180),
        )
        .await;
        let _ = provider.stop_instance().await;
        return Err(format!(
            "GPU 已启动，但无法设置 {hard_limit_minutes} 分钟平台关机保障，已请求关机：{}",
            error.message
        ));
    }
    let instance = wait_for_instance_state(
        &provider,
        CompSharePowerState::Running,
        Some(CompShareRunningMode::Gpu),
        Duration::from_secs(180),
    )
    .await?;
    Ok(CompShareActionResult {
        request_sent: initial.request_sent,
        requested_mode: initial.requested_mode,
        instance,
    })
}

#[tauri::command]
async fn stop_compshare_instance(
    provider: State<'_, CompShareProvider>,
) -> Result<CompShareActionResult, CompShareError> {
    provider.stop_instance().await
}

#[tauri::command]
async fn prepare_application_exit(
    provider: State<'_, CompShareProvider>,
    service: State<'_, ServiceClient>,
    compute: State<'_, ComputeControlStore>,
    lifecycle: State<'_, ComputeLifecycle>,
) -> Result<ApplicationExitProtection, String> {
    lifecycle.invalidate_idle_shutdown();
    let keep_alive_policy = lifecycle.policy();
    let managed_instances = compute.list_instances().map_err(|error| error.message)?;
    let mut gpu_instances = 0_u32;
    let mut stopped_instances = 0_u32;
    let mut protected_instances = 0_u32;
    let mut active_remote_tasks = 0_u64;
    let mut unreachable_instances = 0_u32;
    let snapshot = lifecycle.snapshot();

    for managed in managed_instances {
        if managed.role == ComputeInstanceRole::UserManaged
            || !managed.platform_state.eq_ignore_ascii_case("running")
            || managed.running_mode != "gpu"
        {
            continue;
        }
        let locator = compute_instance_locator(&managed);
        let instance = provider
            .describe_instance(locator.clone())
            .await
            .map_err(|error| {
                format!(
                    "无法确认实例 {} 的退出状态：{}",
                    managed.instance_id, error.message
                )
            })?;
        compute
            .refresh_platform_instance(&instance)
            .map_err(|error| error.message)?;
        if instance.state != CompSharePowerState::Running
            || instance.running_mode != CompShareRunningMode::Gpu
        {
            continue;
        }
        gpu_instances += 1;

        let ensure_watchdog = async {
            if instance.stop_scheduler_time.is_none() {
                provider
                    .update_stop_scheduler_for(
                        locator.clone(),
                        chrono::Utc::now().timestamp() + (snapshot.hard_limit_minutes as i64) * 60,
                    )
                    .await
                    .map_err(|error| {
                        format!(
                            "实例 {} 无法补设平台关机保障：{}",
                            managed.instance_id, error.message
                        )
                    })?;
            }
            Ok::<(), String>(())
        };

        if keep_alive_policy == ComputeKeepAlivePolicy::Continuous {
            ensure_watchdog.await?;
            protected_instances += 1;
            continue;
        }

        let probe = match service.probe_for(&managed.instance_id).await {
            Ok(probe) => probe,
            Err(_) => {
                ensure_watchdog.await?;
                unreachable_instances += 1;
                protected_instances += 1;
                continue;
            }
        };
        let worker_tasks = probe.queue_active.saturating_add(probe.queue_queued);
        active_remote_tasks = active_remote_tasks.saturating_add(u64::from(worker_tasks));
        if worker_tasks > 0 {
            ensure_watchdog.await?;
            protected_instances += 1;
            continue;
        }

        provider
            .stop_instance_for(locator.clone())
            .await
            .map_err(|error| {
                format!(
                    "实例 {} 的远端队列已空，但请求关闭 GPU 失败：{}",
                    managed.instance_id, error.message
                )
            })?;
        let _ = provider.delete_stop_scheduler_for(locator.clone()).await;
        stopped_instances += 1;
    }

    if gpu_instances == 0 {
        return Ok(ApplicationExitProtection {
            active_remote_tasks: 0,
            action: "no-gpu-cost".to_owned(),
            detail: "当前没有运行中的 GPU，无需额外处理。".to_owned(),
        });
    }
    if keep_alive_policy == ComputeKeepAlivePolicy::Continuous {
        return Ok(ApplicationExitProtection {
            active_remote_tasks,
            action: "continuous-gpu".to_owned(),
            detail: format!(
                "持续 GPU 模式保留 {protected_instances} 个实例，并已逐台确认平台硬上限。"
            ),
        });
    }
    let action = if protected_instances > 0 {
        "mixed-worker-protection"
    } else {
        "gpu-stop-requested"
    };
    Ok(ApplicationExitProtection {
        active_remote_tasks,
        action: action.to_owned(),
        detail: format!(
            "已请求关闭 {stopped_instances} 个空闲 GPU；{protected_instances} 个实例因任务运行或队列不可达而保留平台兜底，其中 {unreachable_instances} 个暂时无法确认队列。"
        ),
    })
}

#[tauri::command]
async fn update_compshare_stop_scheduler(
    provider: State<'_, CompShareProvider>,
    input: UpdateCompShareStopSchedulerInput,
) -> Result<CompShareSchedulerResult, CompShareError> {
    provider.update_stop_scheduler(input).await
}

#[tauri::command]
async fn delete_compshare_stop_scheduler(
    provider: State<'_, CompShareProvider>,
) -> Result<comp_share::CompShareDeleteSchedulerResult, CompShareError> {
    provider.delete_stop_scheduler().await
}

#[tauri::command]
fn save_ssh_tunnel_configuration(
    manager: State<'_, SshTunnelManager>,
    input: SaveTunnelConfigurationInput,
) -> Result<TunnelStatus, TunnelError> {
    manager.save_configuration(input)
}

#[tauri::command]
async fn start_ssh_tunnel(
    manager: State<'_, SshTunnelManager>,
    provider: State<'_, CompShareProvider>,
) -> Result<TunnelStatus, TunnelError> {
    let instance_id = tunnel_bound_instance_id(&provider)?;
    manager.start_for(&instance_id).await
}

#[tauri::command]
async fn stop_ssh_tunnel(
    manager: State<'_, SshTunnelManager>,
    provider: State<'_, CompShareProvider>,
) -> Result<TunnelStatus, TunnelError> {
    let instance_id = tunnel_bound_instance_id(&provider)?;
    manager.stop_for(&instance_id).await
}

#[tauri::command]
fn get_ssh_tunnel_status(
    manager: State<'_, SshTunnelManager>,
    provider: State<'_, CompShareProvider>,
) -> Result<TunnelStatus, TunnelError> {
    let instance_id = tunnel_bound_instance_id(&provider)?;
    manager.status_for(&instance_id)
}

fn tunnel_bound_instance_id(provider: &CompShareProvider) -> Result<String, TunnelError> {
    provider
        .configuration()
        .map_err(|error| TunnelError {
            code: error.code,
            message: error.message,
        })?
        .bound_instance_id
        .ok_or_else(|| TunnelError {
            code: "TUNNEL_INSTANCE_NOT_BOUND".to_owned(),
            message: "尚未绑定优云智算实例".to_owned(),
        })
}

async fn connect_service_through_tunnel_impl(
    manager: &SshTunnelManager,
    service: &ServiceClient,
    compute: &ComputeControlStore,
    instance_id: &str,
) -> Result<ServiceProbe, String> {
    let _ = compute.record_worker_readiness(
        instance_id,
        ComputeServiceState::Connecting,
        None,
        None,
        None,
        None,
        Some("正在建立 SSH 隧道并检查知画服务"),
    );
    let result = async {
        let tunnel = manager
            .start_for(instance_id)
            .await
            .map_err(|error| error.message)?;
        let local_url = tunnel
            .local_url
            .ok_or_else(|| "SSH 隧道没有返回本机服务地址".to_owned())?;
        let connection_info = service
            .info_for(instance_id)
            .map_err(|error| error.message)?;
        if connection_info.configured
            && connection_info.credential_stored
            && connection_info.instance_id.as_deref() == Some(instance_id)
        {
            service
                .retarget_for(instance_id, local_url)
                .map_err(|error| error.message)?;
        } else {
            let token = manager
                .read_service_token_for(instance_id)
                .await
                .map_err(|error| error.message)?;
            service
                .save_for(SaveServiceConnectionInput {
                    instance_id: instance_id.to_owned(),
                    base_url: local_url,
                    token,
                })
                .map_err(|error| error.message)?;
        }
        service
            .probe_for(instance_id)
            .await
            .map_err(|error| error.message)
    }
    .await;

    match result {
        Ok(probe) => {
            let state = if !probe.compatible {
                ComputeServiceState::Incompatible
            } else if probe.comfyui_ready {
                ComputeServiceState::Ready
            } else {
                ComputeServiceState::WaitingForGpu
            };
            compute
                .record_worker_readiness(
                    instance_id,
                    state,
                    Some(&probe.service_version),
                    probe.api_version.as_deref(),
                    Some(&probe.workflow_manifest_version),
                    Some(&probe.model_manifest_version),
                    Some(&probe.detail),
                )
                .map_err(|error| error.message)?;
            Ok(probe)
        }
        Err(message) => {
            let _ = compute.record_worker_readiness(
                instance_id,
                ComputeServiceState::Unreachable,
                None,
                None,
                None,
                None,
                Some(&message),
            );
            Err(message)
        }
    }
}

#[tauri::command]
async fn connect_service_through_tunnel(
    manager: State<'_, SshTunnelManager>,
    service: State<'_, ServiceClient>,
    comp_share: State<'_, CompShareProvider>,
    compute: State<'_, ComputeControlStore>,
) -> Result<ServiceProbe, String> {
    let instance_id = comp_share
        .configuration()
        .map_err(|error| error.message)?
        .bound_instance_id
        .ok_or_else(|| "尚未绑定优云智算实例".to_owned())?;
    connect_service_through_tunnel_impl(&manager, &service, &compute, &instance_id).await
}

#[tauri::command]
async fn connect_compute_worker(
    manager: State<'_, SshTunnelManager>,
    service: State<'_, ServiceClient>,
    compute: State<'_, ComputeControlStore>,
    instance_id: String,
) -> Result<ServiceProbe, String> {
    compute
        .get_instance(&instance_id)
        .map_err(|error| error.message)?;
    connect_service_through_tunnel_impl(&manager, &service, &compute, &instance_id).await
}

#[tauri::command]
async fn provision_compute_worker_connection(
    manager: State<'_, SshTunnelManager>,
    service: State<'_, ServiceClient>,
    comp_share: State<'_, CompShareProvider>,
    compute: State<'_, ComputeControlStore>,
    lifecycle: State<'_, ComputeLifecycle>,
    instance_id: String,
) -> Result<ServiceProbe, String> {
    provision_compute_worker_connection_impl(
        &manager,
        &service,
        &comp_share,
        &compute,
        &lifecycle,
        &instance_id,
    )
    .await
}

async fn provision_compute_worker_connection_impl(
    manager: &SshTunnelManager,
    service: &ServiceClient,
    comp_share: &CompShareProvider,
    compute: &ComputeControlStore,
    lifecycle: &ComputeLifecycle,
    instance_id: &str,
) -> Result<ServiceProbe, String> {
    lifecycle.invalidate_idle_shutdown();
    let needs_bootstrap = !manager
        .status_for(instance_id)
        .map_err(|error| error.message)?
        .configured;
    let managed = compute
        .get_instance(&instance_id)
        .map_err(|error| error.message)?;
    if managed.role == ComputeInstanceRole::UserManaged {
        return Err("请先将该实例设为主实例或加入批量算力池".to_owned());
    }
    let locator = compute_instance_locator(&managed);
    let mut started_without_gpu = false;
    let result = async {
        let mut instance = comp_share
            .describe_instance(locator.clone())
            .await
            .map_err(|error| error.message)?;
        if instance.state == CompSharePowerState::Stopping {
            instance = wait_for_instance_state_for(
                &comp_share,
                &locator,
                CompSharePowerState::Stopped,
                None,
                Duration::from_secs(180),
            )
            .await?;
        } else if instance.state == CompSharePowerState::Starting {
            instance = wait_for_instance_state_for(
                &comp_share,
                &locator,
                CompSharePowerState::Running,
                None,
                Duration::from_secs(180),
            )
            .await?;
        }
        if instance.state == CompSharePowerState::Stopped {
            if !instance.support_without_gpu_start {
                return Err("该实例不支持无卡启动；为避免产生 GPU 费用，知画没有启动它".to_owned());
            }
            comp_share
                .start_instance_for(locator.clone(), CompShareStartMode::NoGpu)
                .await
                .map_err(|error| error.message)?;
            started_without_gpu = true;
            if let Err(error) = comp_share
                .update_stop_scheduler_for(
                    locator.clone(),
                    chrono::Utc::now().timestamp() + 30 * 60,
                )
                .await
            {
                let _ = comp_share.stop_instance_for(locator.clone()).await;
                return Err(format!(
                    "无卡实例已请求启动，但平台关机保障设置失败；知画已请求关机：{}",
                    error.message
                ));
            }
            instance = wait_for_instance_state_for(
                &comp_share,
                &locator,
                CompSharePowerState::Running,
                Some(CompShareRunningMode::NoGpu),
                Duration::from_secs(180),
            )
            .await?;
        }
        if instance.state != CompSharePowerState::Running {
            return Err(format!(
                "实例当前状态无法配置安全连接：{}",
                instance.raw_state
            ));
        }
        compute
            .refresh_platform_instance(&instance)
            .map_err(|error| error.message)?;

        if needs_bootstrap {
            let bootstrap_deadline = tokio::time::Instant::now() + Duration::from_secs(150);
            loop {
                let access = match comp_share.ssh_access_for(locator.clone()).await {
                    Ok(access) => access,
                    Err(error)
                        if matches!(
                            error.code.as_str(),
                            "SSH_ENDPOINT_UNAVAILABLE" | "SSH_PASSWORD_UNAVAILABLE"
                        ) && tokio::time::Instant::now() < bootstrap_deadline =>
                    {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        continue;
                    }
                    Err(error) => return Err(error.message),
                };
                match manager
                    .bootstrap_configuration(
                        instance_id,
                        &access.host,
                        access.port,
                        &access.username,
                        &access.password,
                    )
                    .await
                {
                    Ok(_) => break,
                    Err(error)
                        if matches!(
                            error.code.as_str(),
                            "SSH_CONNECT_TIMEOUT" | "SSH_CONNECT_FAILED"
                        ) && tokio::time::Instant::now() < bootstrap_deadline =>
                    {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                    Err(error) => return Err(error.message),
                }
            }
        }

        manager
            .ensure_remote_service_for(&instance_id)
            .await
            .map_err(|error| error.message)?;

        let service_deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        loop {
            match connect_service_through_tunnel_impl(&manager, &service, &compute, &instance_id)
                .await
            {
                Ok(probe) => return Ok(probe),
                Err(error) if tokio::time::Instant::now() < service_deadline => {
                    let _ = manager.stop_for(&instance_id).await;
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    let _ = error;
                }
                Err(error) => return Err(error),
            }
        }
    }
    .await;

    if result.is_err() && started_without_gpu {
        let _ = manager.stop_for(&instance_id).await;
        let _ = comp_share.stop_instance_for(locator).await;
    }
    result
}

#[tauri::command]
async fn prepare_job_result_access(
    manager: State<'_, SshTunnelManager>,
    service: State<'_, ServiceClient>,
    comp_share: State<'_, CompShareProvider>,
    compute: State<'_, ComputeControlStore>,
    lifecycle: State<'_, ComputeLifecycle>,
    queue: State<'_, JobQueueStorage>,
    job_id: String,
) -> Result<ServiceProbe, String> {
    let instance_id = remote_job_instance_id(&queue, &job_id).map_err(|error| error.message)?;
    provision_compute_worker_connection_impl(
        &manager,
        &service,
        &comp_share,
        &compute,
        &lifecycle,
        &instance_id,
    )
    .await
}

#[tauri::command]
async fn prepare_generation_service(
    app: AppHandle,
    manager: State<'_, SshTunnelManager>,
    service: State<'_, ServiceClient>,
    comp_share: State<'_, CompShareProvider>,
    compute: State<'_, ComputeControlStore>,
    lifecycle: State<'_, ComputeLifecycle>,
) -> Result<ServiceProbe, String> {
    let instance_id = comp_share
        .configuration()
        .map_err(|error| error.message)?
        .bound_instance_id
        .ok_or_else(|| "尚未绑定优云智算实例".to_owned())?;
    let result = prepare_compute_worker_impl(
        &manager,
        &service,
        &comp_share,
        &compute,
        &lifecycle,
        &instance_id,
    )
    .await;
    if result.is_ok() {
        restart_idle_gpu_shutdown(&app, &lifecycle);
    }
    result
}

#[tauri::command]
async fn prepare_compute_worker(
    app: AppHandle,
    manager: State<'_, SshTunnelManager>,
    service: State<'_, ServiceClient>,
    comp_share: State<'_, CompShareProvider>,
    compute: State<'_, ComputeControlStore>,
    lifecycle: State<'_, ComputeLifecycle>,
    instance_id: String,
) -> Result<ServiceProbe, String> {
    let result = prepare_compute_worker_impl(
        &manager,
        &service,
        &comp_share,
        &compute,
        &lifecycle,
        &instance_id,
    )
    .await;
    if result.is_ok() {
        restart_idle_gpu_shutdown(&app, &lifecycle);
    }
    result
}

async fn prepare_compute_worker_impl(
    manager: &SshTunnelManager,
    service: &ServiceClient,
    comp_share: &CompShareProvider,
    compute: &ComputeControlStore,
    lifecycle: &ComputeLifecycle,
    instance_id: &str,
) -> Result<ServiceProbe, String> {
    lifecycle.invalidate_idle_shutdown();
    let managed = compute
        .get_instance(instance_id)
        .map_err(|error| error.message)?;
    if managed.role == ComputeInstanceRole::UserManaged {
        return Err("该实例尚未被选为主实例，不能自动启动 GPU".to_owned());
    }
    let tunnel = manager
        .status_for(instance_id)
        .map_err(|error| error.message)?;
    if !tunnel.configured {
        provision_compute_worker_connection_impl(
            manager,
            service,
            comp_share,
            compute,
            lifecycle,
            instance_id,
        )
        .await?;
    }

    let locator = compute_instance_locator(&managed);
    let mut instance = comp_share
        .describe_instance(locator.clone())
        .await
        .map_err(|error| error.message)?;
    compute
        .refresh_platform_instance(&instance)
        .map_err(|error| error.message)?;

    if instance.state == CompSharePowerState::Stopping {
        instance = wait_for_instance_state_for(
            comp_share,
            &locator,
            CompSharePowerState::Stopped,
            None,
            Duration::from_secs(180),
        )
        .await?;
    } else if instance.state == CompSharePowerState::Starting {
        instance = wait_for_instance_state_for(
            comp_share,
            &locator,
            CompSharePowerState::Running,
            None,
            Duration::from_secs(180),
        )
        .await?;
    }

    if instance.running_mode == CompShareRunningMode::NoGpu {
        comp_share
            .stop_instance_for(locator.clone())
            .await
            .map_err(|error| error.message)?;
        instance = wait_for_instance_state_for(
            comp_share,
            &locator,
            CompSharePowerState::Stopped,
            None,
            Duration::from_secs(180),
        )
        .await?;
    }
    if instance.state == CompSharePowerState::Stopped {
        comp_share
            .start_instance_for(locator.clone(), CompShareStartMode::Gpu)
            .await
            .map_err(|error| error.message)?;
    } else if instance.state != CompSharePowerState::Running
        || instance.running_mode != CompShareRunningMode::Gpu
    {
        return Err(format!(
            "实例当前状态无法准备为 GPU worker：{}",
            instance.raw_state
        ));
    }

    let hard_limit_minutes = lifecycle.snapshot().hard_limit_minutes;
    if let Err(error) = comp_share
        .update_stop_scheduler_for(
            locator.clone(),
            chrono::Utc::now().timestamp() + (hard_limit_minutes as i64) * 60,
        )
        .await
    {
        let _ = wait_for_instance_state_for(
            comp_share,
            &locator,
            CompSharePowerState::Running,
            Some(CompShareRunningMode::Gpu),
            Duration::from_secs(180),
        )
        .await;
        let _ = comp_share.stop_instance_for(locator.clone()).await;
        return Err(format!(
            "GPU 已启动，但无法设置 {hard_limit_minutes} 分钟平台关机保障，已请求关机且任务未提交：{}",
            error.message
        ));
    }
    let instance = wait_for_instance_state_for(
        comp_share,
        &locator,
        CompSharePowerState::Running,
        Some(CompShareRunningMode::Gpu),
        Duration::from_secs(180),
    )
    .await?;
    compute
        .refresh_platform_instance(&instance)
        .map_err(|error| error.message)?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(240);
    loop {
        let readiness = match connect_service_through_tunnel_impl(
            manager,
            service,
            compute,
            &instance.instance_id,
        )
        .await
        {
            Ok(probe) if probe.comfyui_ready => return Ok(probe),
            Ok(probe) => probe.detail,
            Err(error) => error,
        };
        if tokio::time::Instant::now() >= deadline {
            let _ = comp_share.stop_instance_for(locator.clone()).await;
            return Err(format!(
                "生成环境未在 4 分钟内就绪，已请求关闭该实例的 GPU：{readiness}"
            ));
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

fn compute_instance_locator(instance: &ManagedComputeInstance) -> CompShareInstanceLocator {
    CompShareInstanceLocator {
        instance_id: instance.instance_id.clone(),
        region: instance.region.clone(),
        zone: instance.zone.clone(),
        project_id: instance.project_id.clone(),
    }
}

async fn wait_for_instance_state(
    provider: &CompShareProvider,
    expected_state: CompSharePowerState,
    expected_mode: Option<CompShareRunningMode>,
    timeout: Duration,
) -> Result<CompShareInstance, String> {
    let configuration = provider.configuration().map_err(|error| error.message)?;
    let locator = CompShareInstanceLocator {
        instance_id: configuration
            .bound_instance_id
            .ok_or_else(|| "尚未绑定优云智算实例".to_owned())?,
        region: configuration
            .region
            .ok_or_else(|| "绑定实例缺少地域".to_owned())?,
        zone: configuration
            .zone
            .ok_or_else(|| "绑定实例缺少可用区".to_owned())?,
        project_id: configuration.project_id,
    };
    wait_for_instance_state_for(provider, &locator, expected_state, expected_mode, timeout).await
}

async fn wait_for_instance_state_for(
    provider: &CompShareProvider,
    locator: &CompShareInstanceLocator,
    expected_state: CompSharePowerState,
    expected_mode: Option<CompShareRunningMode>,
    timeout: Duration,
) -> Result<CompShareInstance, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let platform_detail = match provider.describe_instance(locator.clone()).await {
            Ok(instance) => {
                if instance.state == expected_state
                    && expected_mode.map_or(true, |mode| instance.running_mode == mode)
                {
                    return Ok(instance);
                }
                format!("平台当前返回 {}", instance.raw_state)
            }
            Err(error) => format!("平台查询失败：{}", error.message),
        };
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("实例状态切换超时，{platform_detail}"));
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

#[tauri::command]
fn get_storage_info(
    app: AppHandle,
    storage: State<'_, ProjectStorage>,
) -> Result<StorageInfo, String> {
    storage_info_with_preferences(&app, &storage)
}

#[tauri::command]
fn get_storage_usage(storage: State<'_, ProjectStorage>) -> Result<storage::StorageUsage, String> {
    storage.usage().map_err(|error| error.to_string())
}

#[tauri::command]
fn open_projects_root(storage: State<'_, ProjectStorage>) -> Result<(), String> {
    let path = storage.info().projects_root;
    std::process::Command::new("explorer")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开项目目录：{error}"))
}

#[tauri::command]
fn set_projects_root(
    app: AppHandle,
    storage: State<'_, ProjectStorage>,
    path: String,
) -> Result<StorageInfo, String> {
    let requested = PathBuf::from(path.trim());
    if !requested.is_absolute() {
        return Err("请选择一个有效的绝对目录。".to_owned());
    }
    if requested.parent().is_none() {
        return Err("不能直接把磁盘根目录设为项目目录，请先创建一个知画专用文件夹。".to_owned());
    }
    fs::create_dir_all(&requested).map_err(|error| format!("无法创建项目目录：{error}"))?;
    let requested = requested
        .canonicalize()
        .map_err(|error| format!("无法读取项目目录：{error}"))?;
    save_storage_preferences(
        &app,
        &StoragePreferences {
            projects_root: Some(requested),
        },
    )?;
    storage_info_with_preferences(&app, &storage)
}

#[tauri::command]
fn reset_projects_root(
    app: AppHandle,
    storage: State<'_, ProjectStorage>,
) -> Result<StorageInfo, String> {
    save_storage_preferences(&app, &StoragePreferences::default())?;
    storage_info_with_preferences(&app, &storage)
}

#[tauri::command]
fn create_project(
    storage: State<'_, ProjectStorage>,
    input: CreateProjectInput,
) -> Result<Project, String> {
    storage
        .create_project(input)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn list_projects(storage: State<'_, ProjectStorage>) -> Result<Vec<Project>, String> {
    storage.list_projects().map_err(|error| error.to_string())
}

#[tauri::command]
fn update_project(
    storage: State<'_, ProjectStorage>,
    input: UpdateProjectInput,
) -> Result<Project, String> {
    storage
        .update_project(input)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn rename_project(
    storage: State<'_, ProjectStorage>,
    id: String,
    title: String,
) -> Result<Project, String> {
    storage
        .rename_project(&id, title)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn duplicate_project(
    storage: State<'_, ProjectStorage>,
    id: String,
    title: Option<String>,
) -> Result<Project, String> {
    storage
        .duplicate_project(&id, title)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_project(storage: State<'_, ProjectStorage>, id: String) -> Result<(), String> {
    storage
        .delete_project(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn open_project(storage: State<'_, ProjectStorage>, id: String) -> Result<Project, String> {
    storage
        .mark_project_opened(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn list_sources(
    storage: State<'_, SourceStorage>,
    project_id: String,
) -> Result<Vec<Source>, String> {
    storage
        .list_sources(&project_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn import_source_file(
    storage: State<'_, SourceStorage>,
    input: ImportSourceFileInput,
) -> Result<Source, String> {
    storage
        .import_file(input)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn create_pasted_source(
    storage: State<'_, SourceStorage>,
    input: CreatePastedSourceInput,
) -> Result<Source, String> {
    storage
        .create_pasted(input)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn read_source_text(storage: State<'_, SourceStorage>, id: String) -> Result<String, String> {
    storage.read_text(&id).map_err(|error| error.to_string())
}

#[tauri::command]
fn set_source_enabled(
    storage: State<'_, SourceStorage>,
    input: SetSourceEnabledInput,
) -> Result<Source, String> {
    storage
        .set_enabled(input)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_source(storage: State<'_, SourceStorage>, id: String) -> Result<(), String> {
    storage.delete(&id).map_err(|error| error.to_string())
}

#[tauri::command]
fn get_llm_configuration(provider: State<'_, LlmProvider>) -> Result<LlmConfiguration, LlmError> {
    provider.configuration()
}

#[tauri::command]
async fn save_llm_configuration(
    provider: State<'_, LlmProvider>,
    input: SaveLlmConfigurationInput,
) -> Result<LlmConfiguration, LlmError> {
    provider.verify_and_save_configuration(input).await
}

#[tauri::command]
fn clear_llm_api_key(provider: State<'_, LlmProvider>) -> Result<(), LlmError> {
    provider.clear_api_key()
}

#[tauri::command]
async fn test_llm_connection(
    provider: State<'_, LlmProvider>,
) -> Result<LlmConnectionTest, LlmError> {
    provider.test_connection().await
}

#[tauri::command]
fn knowledge_point_list(
    provider: State<'_, LlmProvider>,
    project_id: String,
) -> Result<Vec<KnowledgePoint>, LlmError> {
    provider.list_knowledge_points(&project_id)
}

#[tauri::command]
fn knowledge_points_replace(
    provider: State<'_, LlmProvider>,
    project_id: String,
    points: Vec<KnowledgePoint>,
) -> Result<Vec<KnowledgePoint>, LlmError> {
    provider.replace_knowledge_points(&project_id, &points)
}

#[tauri::command]
async fn knowledge_extract(
    provider: State<'_, LlmProvider>,
    source_storage: State<'_, SourceStorage>,
    input: AnalyzeSourcesInput,
) -> Result<Vec<KnowledgePoint>, LlmError> {
    let available = source_storage
        .list_sources(&input.project_id)
        .map_err(|error| LlmError {
            code: "SOURCE_ERROR".to_owned(),
            message: error.to_string(),
        })?;
    let requested = input
        .source_ids
        .iter()
        .collect::<std::collections::HashSet<_>>();
    let mut sources = Vec::new();
    for source in available {
        if !requested.contains(&source.id) || !source.enabled || source.extracted_char_count == 0 {
            continue;
        }
        let text = source_storage
            .read_text(&source.id)
            .map_err(|error| LlmError {
                code: "SOURCE_ERROR".to_owned(),
                message: error.to_string(),
            })?;
        sources.push(LlmSource {
            id: source.id,
            name: source.name,
            text,
        });
    }
    provider.analyze_sources(input, sources).await
}

#[tauri::command]
async fn storyboard_generate_from_knowledge(
    provider: State<'_, LlmProvider>,
    storyboard: State<'_, StoryboardStorage>,
    input: CreateStoryboardInput,
) -> Result<Vec<SceneDraft>, LlmError> {
    generate_and_persist_storyboard(provider.inner(), storyboard.inner(), input).await
}

#[tauri::command]
async fn storyboard_revise_scene(
    provider: State<'_, LlmProvider>,
    storyboard: State<'_, StoryboardStorage>,
    input: ReviseSceneInput,
) -> Result<SceneRevisionProposal, LlmError> {
    let scene = storyboard
        .list(&input.project_id)
        .map_err(|error| LlmError {
            code: "STORYBOARD_ERROR".to_owned(),
            message: error.to_string(),
        })?
        .into_iter()
        .find(|scene| scene.id == input.scene_id)
        .ok_or_else(|| LlmError {
            code: "SCENE_NOT_FOUND".to_owned(),
            message: "找不到要修订的分镜".to_owned(),
        })?;
    provider.revise_scene(&scene, &input.instruction).await
}

async fn generate_and_persist_storyboard(
    provider: &LlmProvider,
    storyboard: &StoryboardStorage,
    input: CreateStoryboardInput,
) -> Result<Vec<SceneDraft>, LlmError> {
    if !storyboard
        .list(&input.project_id)
        .map_err(|error| LlmError {
            code: "STORYBOARD_ERROR".to_owned(),
            message: error.to_string(),
        })?
        .is_empty()
    {
        return Err(LlmError {
            code: "STORYBOARD_NOT_EMPTY".to_owned(),
            message: "当前项目已有分镜，为避免覆盖，请先在分镜页处理现有内容。".to_owned(),
        });
    }
    let points = provider.list_knowledge_points(&input.project_id)?;
    let plans = provider.create_storyboard(&input).await?;
    let mut saved = Vec::with_capacity(plans.len());
    for (order, plan) in plans.into_iter().enumerate() {
        let mut seen_sources = std::collections::HashSet::new();
        let source_refs = plan
            .knowledge_point_ids
            .iter()
            .filter_map(|id| points.iter().find(|point| &point.id == id))
            .flat_map(|point| point.source_refs.iter())
            .filter(|reference| seen_sources.insert(reference.source_id.clone()))
            .map(|reference| SourceReference {
                source_id: reference.source_id.clone(),
                page: None,
                paragraph: None,
                quote: None,
            })
            .collect();
        let scene = SceneDraft {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: input.project_id.clone(),
            order: order as u32,
            title: plan.title,
            purpose: plan.purpose,
            source_refs,
            narration: plan.narration,
            narration_mode: NarrationMode::Tts,
            ambient_sound: if plan.ambient_sound.trim().is_empty() {
                "与画面同步的自然环境声".to_owned()
            } else {
                plan.ambient_sound
            },
            on_screen_text: plan.on_screen_text,
            visual_plan: plan.visual_plan,
            visual_intent: crate::storyboard::VisualIntent::default(),
            prompt_mode: crate::storyboard::PromptMode::Quick,
            audio_intent: crate::storyboard::AudioIntent::Environment,
            locked: false,
            generation_mode: GenerationMode::T2v,
            target_duration_ms: plan.target_duration_sec * 1_000,
            asset_ids: Vec::new(),
            selected_version_id: None,
            last_job_id: None,
            last_upscale_job_id: None,
            pending_request_id: None,
            generation_stage: None,
            status: SceneStatus::Draft,
            quality: CandidateQuality::Fast,
            updated_at: String::new(),
        };
        match storyboard.upsert(scene) {
            Ok(scene) => saved.push(scene),
            Err(error) => {
                for inserted in &saved {
                    let _ = storyboard.delete(&input.project_id, &inserted.id);
                }
                return Err(LlmError {
                    code: "STORYBOARD_ERROR".to_owned(),
                    message: error.to_string(),
                });
            }
        }
    }
    Ok(saved)
}

#[cfg(test)]
mod live_flow_tests {
    use super::*;

    #[test]
    fn candidate_frame_profile_comes_from_the_submitted_job_contract() {
        let parameters = serde_json::json!({
            "aspectRatio": "16:9",
            "width": 1344,
            "height": 768,
            "visibleWidth": 1344,
            "visibleHeight": 756,
            "cropX": 0,
            "cropY": 6,
        });
        let profile = generation_frame_profile(&parameters).expect("valid submitted profile");
        assert_eq!(profile.aspect_ratio.label(), "16:9");
        assert_eq!(profile.visible.width, 1344);
        assert_eq!(profile.visible.height, 756);

        let mut tampered = parameters;
        tampered["visibleHeight"] = serde_json::json!(768);
        let error = generation_frame_profile(&tampered).expect_err("mismatch must fail");
        assert!(error.contains("画幅参数"));
    }

    #[test]
    fn compute_policy_persists_and_exposes_bounded_safety_windows() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("compute-policy.json");
        let lifecycle = ComputeLifecycle::load(path.clone());
        assert_eq!(lifecycle.snapshot().policy, ComputeKeepAlivePolicy::Economy);
        assert_eq!(lifecycle.snapshot().idle_shutdown_minutes, Some(3));
        assert_eq!(lifecycle.snapshot().hard_limit_minutes, 60);

        let continuous = lifecycle
            .set_policy(ComputeKeepAlivePolicy::Continuous)
            .expect("persist continuous policy");
        assert_eq!(continuous.idle_shutdown_minutes, None);
        assert_eq!(continuous.hard_limit_minutes, 720);

        let reloaded = ComputeLifecycle::load(path).snapshot();
        assert_eq!(reloaded.policy, ComputeKeepAlivePolicy::Continuous);
        assert_eq!(reloaded.hard_limit_minutes, 720);
    }

    #[tokio::test]
    #[ignore = "requires ZHIHUA_TEST_DEEPSEEK_API_KEY and performs two authorized live DeepSeek requests"]
    async fn live_content_to_persisted_storyboard_flow() {
        let api_key = std::env::var("ZHIHUA_TEST_DEEPSEEK_API_KEY")
            .expect("set ZHIHUA_TEST_DEEPSEEK_API_KEY for the live flow test");
        let directory = tempfile::tempdir().expect("temporary directory");
        let projects = ProjectStorage::initialize(
            directory.path().join("zhihua.sqlite3"),
            directory.path().join("projects"),
        )
        .expect("initialize projects");
        let project = projects
            .create_project(CreateProjectInput {
                title: "雷电形成验收".to_owned(),
                audience: Some("小学高年级".to_owned()),
                target_duration_sec: Some(30),
            })
            .expect("create project");
        let provider = LlmProvider::initialize(
            directory.path().join("settings"),
            projects.info().database_path,
        )
        .expect("initialize DeepSeek provider");
        provider
            .save_configuration(SaveLlmConfigurationInput {
                provider_id: "deepseek".to_owned(),
                protocol: "openai_chat".to_owned(),
                base_url: "https://api.deepseek.com".to_owned(),
                model: "deepseek-chat".to_owned(),
                api_key,
            })
            .expect("save authorized DeepSeek configuration");

        let source_id = "source-lightning".to_owned();
        let points = provider
            .analyze_sources(
                AnalyzeSourcesInput {
                    project_id: project.id.clone(),
                    source_ids: vec![source_id.clone()],
                    target_audience: project.audience.clone(),
                    target_duration_sec: project.target_duration_sec,
                },
                vec![LlmSource {
                    id: source_id.clone(),
                    name: "雷电基础资料.txt".to_owned(),
                    text: "云中的冰晶和水滴碰撞，使云层不同区域积累不同电荷。电势差足够大时，空气被击穿，形成明亮的闪电通道。闪电会把周围空气迅速加热，空气快速膨胀并形成声波，这就是雷声。光传播得比声音快，所以人们通常先看到闪电，后听到雷声。"
                        .to_owned(),
                }],
            )
            .await
            .expect("extract knowledge points from the live model");
        assert!((3..=12).contains(&points.len()));
        assert!(points.iter().all(|point| {
            !point.title.trim().is_empty()
                && !point.detail.trim().is_empty()
                && point
                    .source_refs
                    .iter()
                    .all(|reference| reference.source_id == source_id)
        }));

        let confirmed = points
            .into_iter()
            .map(|mut point| {
                point.confirmed = true;
                point
            })
            .collect::<Vec<_>>();
        provider
            .replace_knowledge_points(&project.id, &confirmed)
            .expect("confirm extracted knowledge points");

        let storyboard = StoryboardStorage::initialize(projects).expect("initialize storyboard");
        let scenes = generate_and_persist_storyboard(
            &provider,
            &storyboard,
            CreateStoryboardInput {
                project_id: project.id.clone(),
                target_audience: project.audience,
                target_duration_sec: project.target_duration_sec,
            },
        )
        .await
        .expect("generate and persist storyboard with the live model");

        assert!(!scenes.is_empty());
        assert!(scenes.iter().all(|scene| {
            !scene.title.trim().is_empty()
                && !scene.narration.trim().is_empty()
                && !scene.visual_plan.trim().is_empty()
                && (4_000..=15_000).contains(&scene.target_duration_ms)
                && scene.target_duration_ms % 1_000 == 0
                && scene.status == SceneStatus::Draft
        }));
        assert_eq!(
            storyboard.list(&project.id).expect("reload storyboard"),
            scenes
        );
    }
}

#[tauri::command]
fn list_storyboard_scenes(
    storage: State<'_, StoryboardStorage>,
    project_id: String,
) -> Result<Vec<SceneDraft>, String> {
    storage.list(&project_id).map_err(|error| error.to_string())
}

#[tauri::command]
fn upsert_storyboard_scene(
    storage: State<'_, StoryboardStorage>,
    scene: SceneDraft,
) -> Result<SceneDraft, String> {
    storage.upsert(scene).map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_storyboard_scene(
    storage: State<'_, StoryboardStorage>,
    project_id: String,
    id: String,
) -> Result<(), String> {
    storage
        .delete(&project_id, &id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn list_system_voices(
    provider: State<'_, SystemTtsProvider>,
) -> Result<Vec<SystemVoice>, TtsError> {
    provider.list_voices()
}

#[tauri::command]
fn get_scene_narration(
    provider: State<'_, SystemTtsProvider>,
    project_id: String,
    scene_id: String,
) -> Result<Option<NarrationArtifact>, TtsError> {
    provider.get_artifact(&project_id, &scene_id)
}

#[tauri::command]
async fn synthesize_scene_narration(
    provider: State<'_, SystemTtsProvider>,
    storyboard: State<'_, StoryboardStorage>,
    input: SynthesizeNarrationInput,
) -> Result<NarrationArtifact, TtsError> {
    let provider = provider.inner().clone();
    let storyboard = storyboard.inner().clone();
    tauri::async_runtime::spawn_blocking(move || provider.synthesize(&storyboard, input))
        .await
        .map_err(|error| TtsError {
            code: "TTS_TASK_ERROR".to_owned(),
            message: format!("系统旁白任务异常结束：{error}"),
        })?
}

#[tauri::command]
async fn import_scene_narration(
    tts: State<'_, SystemTtsProvider>,
    storyboard: State<'_, StoryboardStorage>,
    assets: State<'_, AssetStorage>,
    input: ImportNarrationInput,
) -> Result<NarrationArtifact, TtsError> {
    let tts = tts.inner().clone();
    let storyboard = storyboard.inner().clone();
    let assets = assets.inner().clone();
    tauri::async_runtime::spawn_blocking(move || tts.import_audio(&storyboard, &assets, input))
        .await
        .map_err(|error| TtsError {
            code: "TTS_TASK_ERROR".to_owned(),
            message: format!("旁白录音导入任务异常结束：{error}"),
        })?
}

#[tauri::command]
fn inspect_export_capability(exporter: State<'_, FfmpegExporter>) -> ExportCapability {
    exporter.capability()
}

#[tauri::command]
async fn export_project_video(
    exporter: State<'_, FfmpegExporter>,
    projects: State<'_, ProjectStorage>,
    storyboard: State<'_, StoryboardStorage>,
    generations: State<'_, GenerationStorage>,
    assets: State<'_, AssetStorage>,
    tts: State<'_, SystemTtsProvider>,
    input: ExportProjectInput,
) -> Result<ProjectExport, ExportError> {
    let project_id = input.project_id.clone();
    let exporter = exporter.inner().clone();
    let storyboard = storyboard.inner().clone();
    let generations = generations.inner().clone();
    let assets = assets.inner().clone();
    let tts = tts.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        exporter.export(&storyboard, &generations, &assets, &tts, input)
    })
    .await
    .map_err(|error| ExportError {
        code: "EXPORT_TASK_ERROR".to_owned(),
        message: format!("导出任务异常结束：{error}"),
    })??;
    let _ = projects.update_project(UpdateProjectInput {
        id: project_id,
        title: None,
        audience: None,
        clear_audience: false,
        target_duration_sec: None,
        clear_target_duration: false,
        status: Some(ProjectStatus::Completed),
        style_profile: None,
    });
    Ok(result)
}

#[tauri::command]
fn reorder_storyboard_scenes(
    storage: State<'_, StoryboardStorage>,
    input: ReorderScenesInput,
) -> Result<Vec<SceneDraft>, String> {
    storage.reorder(input).map_err(|error| error.to_string())
}

#[tauri::command]
async fn test_service_connection(
    service: State<'_, ServiceClient>,
    base_url: String,
) -> Result<ServiceConnectionResult, ServiceConnectionError> {
    service.test_connection(&base_url).await
}

#[tauri::command]
fn get_service_connection_info(
    service: State<'_, ServiceClient>,
) -> Result<ServiceConnectionInfo, ServiceConnectionError> {
    service.info()
}

#[tauri::command]
fn save_service_connection(
    service: State<'_, ServiceClient>,
    input: SaveServiceConnectionInput,
) -> Result<ServiceConnectionInfo, ServiceConnectionError> {
    service.save(input)
}

#[tauri::command]
fn clear_service_connection(
    service: State<'_, ServiceClient>,
) -> Result<(), ServiceConnectionError> {
    service.clear()
}

#[tauri::command]
async fn probe_service(
    service: State<'_, ServiceClient>,
) -> Result<ServiceProbe, ServiceConnectionError> {
    service.probe().await
}

#[tauri::command]
async fn submit_service_job(
    app: AppHandle,
    service: State<'_, ServiceClient>,
    queue: State<'_, JobQueueStorage>,
    compute: State<'_, ComputeControlStore>,
    lifecycle: State<'_, ComputeLifecycle>,
    input: SubmitServiceJobInput,
) -> Result<ServiceJob, ServiceConnectionError> {
    let request_id = input.client_request_id.clone();
    let result = async {
        queue.stage(&input).map_err(queue_service_error)?;
        let lease = match compute.worker_lease(&request_id) {
            Ok(lease) => lease,
            Err(error) if error.code == "WORKER_LEASE_NOT_FOUND" => {
                let primary = compute.primary_instance().map_err(compute_service_error)?;
                let worker_id = format!("instance:{}:gpu:0", primary.instance_id);
                compute
                    .acquire_worker_lease(&worker_id, &primary.instance_id, &request_id, 3_600)
                    .map_err(compute_service_error)?
            }
            Err(error) => return Err(compute_service_error(error)),
        };
        let worker_id = lease.worker_id.clone();
        if let Err(error) = queue.claim_for_worker(&request_id, &worker_id, 3_600) {
            let _ = compute.release_worker_lease(&request_id);
            return Err(queue_service_error(error));
        }
        match service.submit_job_for(&lease.instance_id, input).await {
            Ok(job) => {
                queue.record_remote(&job).map_err(queue_service_error)?;
                Ok(job)
            }
            Err(error) => {
                let _ = queue.record_submit_failure(&request_id, error.code, &error.message);
                let _ = compute.release_worker_lease(&request_id);
                Err(error)
            }
        }
    }
    .await;
    if result.is_err() {
        restart_idle_gpu_shutdown(&app, &lifecycle);
    }
    result
}

#[tauri::command]
async fn get_service_job(
    app: AppHandle,
    service: State<'_, ServiceClient>,
    queue: State<'_, JobQueueStorage>,
    compute: State<'_, ComputeControlStore>,
    lifecycle: State<'_, ComputeLifecycle>,
    job_id: String,
) -> Result<ServiceJob, ServiceConnectionError> {
    let instance_id = remote_job_instance_id(&queue, &job_id)?;
    let job = service.get_job_for(&instance_id, &job_id).await?;
    let local = queue.sync(&job).map_err(queue_service_error)?;
    reconcile_service_job_lease(&app, &queue, &compute, &lifecycle, &local, &job)?;
    Ok(job)
}

fn reconcile_service_job_lease(
    app: &AppHandle,
    _queue: &JobQueueStorage,
    compute: &ComputeControlStore,
    lifecycle: &ComputeLifecycle,
    local: &LocalJob,
    job: &ServiceJob,
) -> Result<(), ServiceConnectionError> {
    if job.status == "completed" {
        if let Err(error) = compute.renew_worker_lease(&job.client_request_id, 900) {
            if error.code != "WORKER_LEASE_NOT_FOUND" {
                return Err(compute_service_error(error));
            }
            let worker_id = local
                .worker_id
                .as_deref()
                .ok_or_else(|| ServiceConnectionError {
                    code: "compute_control_error",
                    message: "已完成的远端任务缺少 worker 分配记录".to_owned(),
                })?;
            let instance_id = worker_instance_id(worker_id)?;
            compute
                .acquire_worker_lease(worker_id, instance_id, &job.client_request_id, 900)
                .map_err(compute_service_error)?;
        }
    } else if is_terminal_service_job(&job) {
        compute
            .release_worker_lease(&job.client_request_id)
            .map_err(compute_service_error)?;
        restart_idle_gpu_shutdown(&app, &lifecycle);
    } else if let Err(error) = compute.renew_worker_lease(&job.client_request_id, 3_600) {
        if error.code != "WORKER_LEASE_NOT_FOUND" {
            return Err(compute_service_error(error));
        }
        let worker_id = local
            .worker_id
            .as_deref()
            .ok_or_else(|| ServiceConnectionError {
                code: "compute_control_error",
                message: "运行中的任务缺少 worker 分配记录".to_owned(),
            })?;
        let instance_id = worker_instance_id(worker_id)?;
        compute
            .acquire_worker_lease(worker_id, instance_id, &job.client_request_id, 3_600)
            .map_err(compute_service_error)?;
    }
    Ok(())
}

#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationRecoveryReport {
    inspected: usize,
    synchronized: usize,
    downloads_ready_to_resume: usize,
    unreachable: usize,
}

async fn recover_generation_jobs(app: &AppHandle) -> GenerationRecoveryReport {
    let queue = app.state::<JobQueueStorage>();
    let service = app.state::<ServiceClient>();
    let compute = app.state::<ComputeControlStore>();
    let lifecycle = app.state::<ComputeLifecycle>();
    let mut report = GenerationRecoveryReport::default();
    let Ok(jobs) = queue.list(None) else {
        return report;
    };
    for local in jobs {
        let Some(remote_job_id) = local.remote_job_id.as_deref() else {
            continue;
        };
        if matches!(
            local.status.as_str(),
            "completed_local" | "failed" | "cancelled" | "interrupted"
        ) {
            continue;
        }
        report.inspected += 1;
        if local.status == "downloading" {
            if queue
                .mark_download_retry(remote_job_id, "客户端上次在保存结果时退出")
                .is_ok()
            {
                report.downloads_ready_to_resume += 1;
            }
        }
        let Ok(instance_id) = remote_job_instance_id(&queue, remote_job_id) else {
            report.unreachable += 1;
            continue;
        };
        let Ok(job) = service.get_job_for(&instance_id, remote_job_id).await else {
            report.unreachable += 1;
            continue;
        };
        let Ok(synced) = queue.sync(&job) else {
            report.unreachable += 1;
            continue;
        };
        if reconcile_service_job_lease(app, &queue, &compute, &lifecycle, &synced, &job).is_ok() {
            report.synchronized += 1;
        }
    }
    report
}

#[tauri::command]
async fn cancel_service_job(
    app: AppHandle,
    service: State<'_, ServiceClient>,
    queue: State<'_, JobQueueStorage>,
    compute: State<'_, ComputeControlStore>,
    lifecycle: State<'_, ComputeLifecycle>,
    job_id: String,
) -> Result<ServiceJob, ServiceConnectionError> {
    let instance_id = remote_job_instance_id(&queue, &job_id)?;
    let job = service.cancel_job_for(&instance_id, &job_id).await?;
    queue.sync(&job).map_err(queue_service_error)?;
    compute
        .release_worker_lease(&job.client_request_id)
        .map_err(compute_service_error)?;
    restart_idle_gpu_shutdown(&app, &lifecycle);
    Ok(job)
}

#[tauri::command]
fn list_local_jobs(
    queue: State<'_, JobQueueStorage>,
    project_id: Option<String>,
) -> Result<Vec<LocalJob>, JobQueueError> {
    queue.list(project_id.as_deref())
}

fn queue_service_error(error: JobQueueError) -> ServiceConnectionError {
    ServiceConnectionError {
        code: "local_queue_error",
        message: error.message,
    }
}

fn compute_service_error(error: ComputeControlError) -> ServiceConnectionError {
    ServiceConnectionError {
        code: "compute_control_error",
        message: format!("{}：{}", error.code, error.message),
    }
}

fn worker_instance_id(worker_id: &str) -> Result<&str, ServiceConnectionError> {
    worker_id
        .strip_prefix("instance:")
        .and_then(|value| value.strip_suffix(":gpu:0"))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ServiceConnectionError {
            code: "compute_control_error",
            message: "worker 分配记录无法映射到算力实例".to_owned(),
        })
}

fn remote_job_instance_id(
    queue: &JobQueueStorage,
    remote_job_id: &str,
) -> Result<String, ServiceConnectionError> {
    let local = queue
        .find_by_remote(remote_job_id)
        .map_err(queue_service_error)?;
    let worker_id = local
        .worker_id
        .as_deref()
        .ok_or_else(|| ServiceConnectionError {
            code: "compute_control_error",
            message: "远端任务缺少持久化的 worker 分配记录".to_owned(),
        })?;
    worker_instance_id(worker_id).map(str::to_owned)
}

#[tauri::command]
fn plan_generation_compute_pool(input: ComputePoolPlanInput) -> ComputePoolPlan {
    plan_compute_pool(input)
}

#[tauri::command]
fn reserve_service_worker(
    compute: State<'_, ComputeControlStore>,
    service: State<'_, ServiceClient>,
    client_request_id: String,
) -> Result<ComputeWorkerLease, ServiceConnectionError> {
    let candidate_count = compute
        .list_worker_readiness()
        .map_err(compute_service_error)?
        .len()
        .max(1);
    for _ in 0..candidate_count {
        let lease = compute
            .acquire_ready_worker_lease(&client_request_id, 3_600)
            .map_err(compute_service_error)?;
        match service.info_for(&lease.instance_id) {
            Ok(connected) if connected.configured && connected.credential_stored => {
                return Ok(lease);
            }
            Ok(_) | Err(_) => {
                let _ = compute.release_worker_lease(&client_request_id);
                let _ = compute.record_worker_readiness(
                    &lease.instance_id,
                    ComputeServiceState::Unreachable,
                    None,
                    None,
                    None,
                    None,
                    Some("实例服务连接尚未配置，已从本次 worker 候选中移除"),
                );
            }
        }
    }
    Err(ServiceConnectionError {
        code: "worker_connection_missing",
        message: "当前就绪 worker 都缺少独立服务连接，请刷新实例状态后重试。".to_owned(),
    })
}

#[tauri::command]
fn release_service_worker_reservation(
    compute: State<'_, ComputeControlStore>,
    client_request_id: String,
) -> Result<bool, ComputeControlError> {
    compute.release_worker_lease(&client_request_id)
}

#[tauri::command]
async fn upload_service_input(
    service: State<'_, ServiceClient>,
    compute: State<'_, ComputeControlStore>,
    source_path: String,
    client_request_id: Option<String>,
    instance_id: Option<String>,
) -> Result<ServiceInputUpload, ServiceConnectionError> {
    if let Some(request_id) = client_request_id {
        let lease = compute
            .worker_lease(&request_id)
            .map_err(compute_service_error)?;
        if instance_id
            .as_deref()
            .is_some_and(|value| value != lease.instance_id)
        {
            return Err(ServiceConnectionError {
                code: "worker_route_mismatch",
                message: "素材目标实例与任务预留的 worker 不一致。".to_owned(),
            });
        }
        return service
            .upload_input_for(&lease.instance_id, &source_path)
            .await;
    }
    if let Some(instance_id) = instance_id {
        return service.upload_input_for(&instance_id, &source_path).await;
    }
    service.upload_input(&source_path).await
}

#[tauri::command]
async fn delete_service_input(
    service: State<'_, ServiceClient>,
    compute: State<'_, ComputeControlStore>,
    input_id: String,
    client_request_id: Option<String>,
    instance_id: Option<String>,
) -> Result<(), ServiceConnectionError> {
    if let Some(request_id) = client_request_id {
        match compute.worker_lease(&request_id) {
            Ok(lease) => {
                if instance_id
                    .as_deref()
                    .is_some_and(|value| value != lease.instance_id)
                {
                    return Err(ServiceConnectionError {
                        code: "worker_route_mismatch",
                        message: "素材所在实例与任务预留的 worker 不一致。".to_owned(),
                    });
                }
                return service
                    .delete_input_for(&lease.instance_id, &input_id)
                    .await;
            }
            Err(error) if error.code == "WORKER_LEASE_NOT_FOUND" => {}
            Err(error) => return Err(compute_service_error(error)),
        }
    }
    if let Some(instance_id) = instance_id {
        return service.delete_input_for(&instance_id, &input_id).await;
    }
    service.delete_input(&input_id).await
}

#[tauri::command]
async fn download_service_artifact(
    service: State<'_, ServiceClient>,
    queue: State<'_, JobQueueStorage>,
    input: DownloadServiceArtifactInput,
) -> Result<ServiceArtifactDownload, ServiceConnectionError> {
    let instance_id = remote_job_instance_id(&queue, &input.job_id)?;
    service.download_artifact_for(&instance_id, input).await
}

#[tauri::command]
fn apply_storyboard_edit(
    storage: State<'_, StoryboardStorage>,
    input: storyboard::StoryboardEditInput,
) -> Result<Vec<SceneDraft>, String> {
    storage.apply_edit(input).map_err(|error| error.to_string())
}

#[tauri::command]
fn list_candidate_versions(
    storage: State<'_, GenerationStorage>,
    input: CandidateVersionsInput,
) -> Result<Vec<CandidateVersion>, GenerationError> {
    storage.list(&input.project_id, &input.scene_id)
}

#[tauri::command]
fn list_enhanced_versions(
    storage: State<'_, GenerationStorage>,
    input: CandidateVersionsInput,
) -> Result<Vec<EnhancedVersion>, GenerationError> {
    storage.list_enhanced(&input.project_id, &input.scene_id)
}

#[tauri::command]
fn inspect_candidate_audio(
    storage: State<'_, GenerationStorage>,
    input: InspectCandidateAudioInput,
) -> Result<CandidateAudioInspection, String> {
    audio_inspector::inspect_candidate(storage.inner(), input)
}

#[tauri::command]
fn open_candidate_location(
    storage: State<'_, GenerationStorage>,
    candidate_id: String,
) -> Result<(), String> {
    let local_path = storage
        .find(&candidate_id)
        .map(|candidate| candidate.local_path)
        .or_else(|_| {
            storage
                .find_enhanced(&candidate_id)
                .map(|enhanced| enhanced.local_path)
        })
        .map_err(|error| error.message)?;
    if !local_path.is_file() {
        return Err("候选文件已移动或不存在。".to_owned());
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", local_path.to_string_lossy()))
            .spawn()
            .map_err(|error| format!("无法打开文件位置：{error}"))?;
    }
    Ok(())
}

#[tauri::command]
fn select_candidate_version(
    storage: State<'_, GenerationStorage>,
    input: SelectCandidateVersionInput,
) -> Result<CandidateVersion, GenerationError> {
    storage.select(&input.project_id, &input.scene_id, &input.version_id)
}

async fn download_artifact_with_queue_progress(
    service: &ServiceClient,
    queue: &JobQueueStorage,
    instance_id: &str,
    remote_job_id: &str,
    input: DownloadServiceArtifactInput,
    completed_before: u64,
    total_bytes: u64,
) -> Result<ServiceArtifactDownload, String> {
    queue
        .mark_downloading(remote_job_id, completed_before, total_bytes)
        .map_err(|error| error.message)?;
    let progress_queue = queue.clone();
    let progress_job_id = remote_job_id.to_owned();
    let result = service
        .download_artifact_for_with_progress(instance_id, input, move |received, _| {
            let _ = progress_queue.mark_downloading(
                &progress_job_id,
                completed_before.saturating_add(received).min(total_bytes),
                total_bytes,
            );
        })
        .await;
    match result {
        Ok(downloaded) => Ok(downloaded),
        Err(error) => {
            let _ = queue.mark_download_retry(remote_job_id, &error.message);
            Err(format!(
                "{} 远端结果仍保留，可稍后继续下载。",
                error.message
            ))
        }
    }
}

#[tauri::command]
async fn download_completed_job(
    app: AppHandle,
    projects: State<'_, ProjectStorage>,
    storyboards: State<'_, StoryboardStorage>,
    generations: State<'_, GenerationStorage>,
    queue: State<'_, JobQueueStorage>,
    compute: State<'_, ComputeControlStore>,
    service: State<'_, ServiceClient>,
    lifecycle: State<'_, ComputeLifecycle>,
    input: DownloadCompletedJobInput,
) -> Result<Vec<CandidateVersion>, String> {
    let project = projects
        .get_project(&input.project_id)
        .map_err(|error| error.to_string())?;
    let instance_id =
        remote_job_instance_id(&queue, &input.job_id).map_err(|error| error.message)?;
    let job = service
        .get_job_for(&instance_id, &input.job_id)
        .await
        .map_err(|error| error.message)?;
    if job.project_id != project.id {
        return Err("远端任务不属于当前项目，已停止下载。".to_owned());
    }
    if !matches!(
        job.kind.as_str(),
        "video_candidate" | "video_reference_remake"
    ) {
        return Err("远端任务不是候选视频任务，已停止下载。".to_owned());
    }
    if !storyboards
        .list(&project.id)
        .map_err(|error| error.to_string())?
        .iter()
        .any(|scene| scene.id == job.scene_id)
    {
        return Err("远端任务对应的本地分镜不存在，已停止下载。".to_owned());
    }
    if job.status != "completed" {
        return Err("远端任务尚未完成，暂时不能下载候选视频。".to_owned());
    }
    let manifest = job
        .result_manifest
        .clone()
        .ok_or_else(|| "远端任务已完成，但没有返回成品清单。".to_owned())?;
    if manifest.job_id != job.id || manifest.workflow_id != job.workflow_id {
        return Err("远端成品清单与任务不匹配，已停止下载。".to_owned());
    }
    let artifacts = manifest
        .artifacts
        .into_iter()
        .filter(|artifact| artifact.kind == "video")
        .collect::<Vec<_>>();
    if artifacts.is_empty() {
        return Err("远端任务没有生成可下载的视频。".to_owned());
    }
    if artifacts.len() > 8 {
        return Err("远端任务返回的视频数量异常，已停止自动下载。".to_owned());
    }
    let generation_parameters = queue
        .request_parameters(&job.id)
        .unwrap_or(serde_json::Value::Null);
    let frame_profile = generation_frame_profile(&generation_parameters)?;
    let prompt_compiler_version = generation_parameters
        .get("promptCompilerVersion")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let h3_audio_policy = generation_parameters
        .get("h3AudioPolicy")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let prompt_text = generation_parameters
        .get("prompt")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let seed = generation_parameters
        .get("seed")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    let audio_intent = generation_parameters
        .get("audioIntent")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let target_duration_sec = generation_parameters
        .get("targetDurationSec")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());

    let total_bytes = artifacts.iter().map(|artifact| artifact.size_bytes).sum();
    let mut completed_bytes = 0_u64;
    let mut candidates = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        let filename = safe_artifact_filename(&artifact.artifact_id, &artifact.filename)?;
        let destination = project
            .project_dir
            .join("generated")
            .join("videos")
            .join(safe_path_component(&job.scene_id)?)
            .join(safe_path_component(&job.id)?)
            .join(&filename);
        let downloaded = download_artifact_with_queue_progress(
            &service,
            &queue,
            &instance_id,
            &job.id,
            DownloadServiceArtifactInput {
                job_id: job.id.clone(),
                artifact_id: artifact.artifact_id.clone(),
                destination_path: destination.to_string_lossy().into_owned(),
                expected_size_bytes: artifact.size_bytes,
                expected_sha256: artifact.sha256.clone(),
            },
            completed_bytes,
            total_bytes,
        )
        .await?;
        completed_bytes = completed_bytes.saturating_add(downloaded.size_bytes);
        candidates.push(
            generations
                .record(RecordCandidateInput {
                    project_id: project.id.clone(),
                    scene_id: job.scene_id.clone(),
                    job_id: job.id.clone(),
                    workflow_id: job.workflow_id.clone(),
                    prompt_id: job.prompt_id.clone(),
                    prompt_compiler_version: prompt_compiler_version.clone(),
                    h3_audio_policy: h3_audio_policy.clone(),
                    prompt_text: prompt_text.clone(),
                    seed,
                    audio_intent: audio_intent.clone(),
                    target_duration_sec,
                    artifact_id: artifact.artifact_id,
                    filename,
                    media_type: artifact.media_type,
                    local_path: downloaded.destination_path.into(),
                    size_bytes: downloaded.size_bytes,
                    sha256: downloaded.sha256,
                    aspect_ratio: frame_profile.aspect_ratio.label().to_owned(),
                    work_width: frame_profile.work.width,
                    work_height: frame_profile.work.height,
                    visible_width: frame_profile.visible.width,
                    visible_height: frame_profile.visible.height,
                    crop_x: frame_profile.crop_x,
                    crop_y: frame_profile.crop_y,
                })
                .map_err(|error| error.message)?,
        );
    }
    finish_result_transfer(&app, &compute, &queue, &lifecycle, &job)?;
    Ok(candidates)
}

#[tauri::command]
async fn preview_project_video(
    exporter: State<'_, FfmpegExporter>,
    storyboard: State<'_, StoryboardStorage>,
    generations: State<'_, GenerationStorage>,
    assets: State<'_, AssetStorage>,
    tts: State<'_, SystemTtsProvider>,
    input: PreviewProjectInput,
) -> Result<ProjectExport, ExportError> {
    let exporter = exporter.inner().clone();
    let storyboard = storyboard.inner().clone();
    let generations = generations.inner().clone();
    let assets = assets.inner().clone();
    let tts = tts.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        exporter.preview(&storyboard, &generations, &assets, &tts, input)
    })
    .await
    .map_err(|error| ExportError {
        code: "PREVIEW_TASK_ERROR".to_owned(),
        message: format!("全片预览任务异常结束：{error}"),
    })?
}

#[tauri::command]
async fn download_completed_image_job(
    app: AppHandle,
    projects: State<'_, ProjectStorage>,
    assets: State<'_, AssetStorage>,
    queue: State<'_, JobQueueStorage>,
    compute: State<'_, ComputeControlStore>,
    service: State<'_, ServiceClient>,
    lifecycle: State<'_, ComputeLifecycle>,
    input: DownloadCompletedImageInput,
) -> Result<Vec<AssetItem>, String> {
    let project = projects
        .get_project(&input.project_id)
        .map_err(|error| error.to_string())?;
    let instance_id =
        remote_job_instance_id(&queue, &input.job_id).map_err(|error| error.message)?;
    let job = service
        .get_job_for(&instance_id, &input.job_id)
        .await
        .map_err(|error| error.message)?;
    if job.project_id != project.id {
        return Err("远端图片任务不属于当前项目，已停止下载。".to_owned());
    }
    let source_label = match job.kind.as_str() {
        "image_generation" => "知画生成",
        "image_edit" => "知画编辑",
        _ => return Err("远端任务不是图片生成或编辑任务。".to_owned()),
    };
    if job.status != "completed" {
        return Err("远端图片任务尚未完成。".to_owned());
    }
    let manifest = job
        .result_manifest
        .clone()
        .ok_or_else(|| "远端图片任务已完成，但没有返回成品清单。".to_owned())?;
    if manifest.job_id != job.id || manifest.workflow_id != job.workflow_id {
        return Err("远端图片清单与任务不匹配，已停止下载。".to_owned());
    }
    let artifacts = manifest
        .artifacts
        .into_iter()
        .filter(|artifact| artifact.kind == "image")
        .collect::<Vec<_>>();
    if artifacts.is_empty() || artifacts.len() > 4 {
        return Err("远端任务返回的图片数量异常。".to_owned());
    }

    let total_bytes = artifacts.iter().map(|artifact| artifact.size_bytes).sum();
    let mut completed_bytes = 0_u64;
    let mut imported = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        let filename = safe_artifact_filename(&artifact.artifact_id, &artifact.filename)?;
        let temporary = project
            .project_dir
            .join("cache")
            .join("image-jobs")
            .join(safe_path_component(&job.id)?)
            .join(filename);
        let downloaded = download_artifact_with_queue_progress(
            &service,
            &queue,
            &instance_id,
            &job.id,
            DownloadServiceArtifactInput {
                job_id: job.id.clone(),
                artifact_id: artifact.artifact_id,
                destination_path: temporary.to_string_lossy().into_owned(),
                expected_size_bytes: artifact.size_bytes,
                expected_sha256: artifact.sha256,
            },
            completed_bytes,
            total_bytes,
        )
        .await?;
        completed_bytes = completed_bytes.saturating_add(downloaded.size_bytes);
        let result = assets
            .import_generated_file(
                &project.id,
                std::path::Path::new(&downloaded.destination_path),
                source_label,
            )
            .map_err(|error| error.message);
        let _ = fs::remove_file(&downloaded.destination_path);
        imported.push(result?);
    }
    finish_result_transfer(&app, &compute, &queue, &lifecycle, &job)?;
    Ok(imported)
}

#[tauri::command]
async fn download_completed_enhancement(
    app: AppHandle,
    projects: State<'_, ProjectStorage>,
    storyboards: State<'_, StoryboardStorage>,
    generations: State<'_, GenerationStorage>,
    queue: State<'_, JobQueueStorage>,
    compute: State<'_, ComputeControlStore>,
    service: State<'_, ServiceClient>,
    lifecycle: State<'_, ComputeLifecycle>,
    input: DownloadCompletedUpscaleInput,
) -> Result<Vec<EnhancedVersion>, String> {
    let project = projects
        .get_project(&input.project_id)
        .map_err(|error| error.to_string())?;
    let instance_id =
        remote_job_instance_id(&queue, &input.job_id).map_err(|error| error.message)?;
    let source = generations
        .find(&input.source_candidate_id)
        .map_err(|error| error.message)?;
    if source.project_id != project.id || !source.selected {
        return Err("1080p 增强任务的来源不是当前项目的正式版本。".to_owned());
    }
    let scene = storyboards
        .list(&project.id)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|scene| scene.id == source.scene_id)
        .ok_or_else(|| "1080p 增强任务对应的本地分镜不存在。".to_owned())?;
    if scene.selected_version_id.as_deref() != Some(source.id.as_str()) {
        return Err("正式版本已经变化，已停止保存旧任务的 1080p 增强版。".to_owned());
    }
    let job = service
        .get_job_for(&instance_id, &input.job_id)
        .await
        .map_err(|error| error.message)?;
    if job.project_id != project.id || job.scene_id != scene.id || job.kind != "video_upscale" {
        return Err("远端 1080p 增强任务与当前正式版本不匹配。".to_owned());
    }
    if job.workflow_id != "seedvr2-1080p-v1" {
        return Err("远端任务没有使用 SeedVR2 1080p 增强工作流。".to_owned());
    }
    if job.status != "completed" {
        return Err("1080p 增强任务尚未完成，暂时不能下载。".to_owned());
    }
    let manifest = job
        .result_manifest
        .clone()
        .ok_or_else(|| "1080p 增强任务已完成，但没有返回文件清单。".to_owned())?;
    if manifest.job_id != job.id || manifest.workflow_id != job.workflow_id {
        return Err("1080p 增强文件清单与任务不匹配，已停止下载。".to_owned());
    }
    let artifacts = manifest
        .artifacts
        .into_iter()
        .filter(|artifact| artifact.kind == "video")
        .collect::<Vec<_>>();
    if artifacts.is_empty() {
        return Err("SeedVR2 任务没有生成可下载的视频。".to_owned());
    }
    if artifacts.len() > 8 {
        return Err("SeedVR2 任务返回的视频数量异常，已停止自动下载。".to_owned());
    }

    let total_bytes = artifacts.iter().map(|artifact| artifact.size_bytes).sum();
    let mut completed_bytes = 0_u64;
    let mut enhanced_versions = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        let filename = safe_artifact_filename(&artifact.artifact_id, &artifact.filename)?;
        let destination = project
            .project_dir
            .join("generated")
            .join("videos")
            .join(safe_path_component(&scene.id)?)
            .join("1080p")
            .join(safe_path_component(&job.id)?)
            .join(&filename);
        let downloaded = download_artifact_with_queue_progress(
            &service,
            &queue,
            &instance_id,
            &job.id,
            DownloadServiceArtifactInput {
                job_id: job.id.clone(),
                artifact_id: artifact.artifact_id.clone(),
                destination_path: destination.to_string_lossy().into_owned(),
                expected_size_bytes: artifact.size_bytes,
                expected_sha256: artifact.sha256.clone(),
            },
            completed_bytes,
            total_bytes,
        )
        .await?;
        completed_bytes = completed_bytes.saturating_add(downloaded.size_bytes);
        enhanced_versions.push(
            generations
                .record_enhanced(RecordEnhancedInput {
                    project_id: project.id.clone(),
                    scene_id: scene.id.clone(),
                    source_candidate_id: source.id.clone(),
                    job_id: job.id.clone(),
                    workflow_id: job.workflow_id.clone(),
                    prompt_id: job.prompt_id.clone(),
                    artifact_id: artifact.artifact_id,
                    filename,
                    media_type: artifact.media_type,
                    local_path: downloaded.destination_path.into(),
                    size_bytes: downloaded.size_bytes,
                    sha256: downloaded.sha256,
                })
                .map_err(|error| error.message)?,
        );
    }
    finish_result_transfer(&app, &compute, &queue, &lifecycle, &job)?;
    Ok(enhanced_versions)
}

fn finish_result_transfer(
    app: &AppHandle,
    compute: &ComputeControlStore,
    queue: &JobQueueStorage,
    lifecycle: &ComputeLifecycle,
    job: &ServiceJob,
) -> Result<(), String> {
    compute
        .release_worker_lease(&job.client_request_id)
        .map_err(|error| error.message)?;
    queue
        .mark_local_complete(&job.id)
        .map_err(|error| error.message)?;
    restart_idle_gpu_shutdown(app, lifecycle);

    let Ok(managed) = compute
        .get_instance(&remote_job_instance_id(queue, &job.id).map_err(|error| error.message)?)
    else {
        return Ok(());
    };
    if managed.running_mode == "no_gpu" {
        schedule_result_instance_cleanup(app, managed.instance_id);
    }
    Ok(())
}

fn schedule_result_instance_cleanup(app: &AppHandle, instance_id: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let manager = app.state::<SshTunnelManager>();
        let provider = app.state::<CompShareProvider>();
        let compute = app.state::<ComputeControlStore>();
        let queue = app.state::<JobQueueStorage>();
        let Ok(managed) = compute.get_instance(&instance_id) else {
            return;
        };
        if managed.running_mode != "no_gpu" {
            return;
        }
        let _ = manager.stop_for(&instance_id).await;
        let locator = compute_instance_locator(&managed);
        if provider.stop_instance_for(locator.clone()).await.is_err() {
            return;
        }
        let Ok(stopped) = wait_for_instance_state_for(
            &provider,
            &locator,
            CompSharePowerState::Stopped,
            None,
            Duration::from_secs(120),
        )
        .await
        else {
            return;
        };
        let Ok(refreshed) = compute.refresh_platform_instance(&stopped) else {
            return;
        };
        let _ = provider.delete_stop_scheduler_for(locator).await;
        if refreshed.role == ComputeInstanceRole::Elastic {
            let _ = release_idle_elastic_instance(&provider, &compute, &queue, &refreshed).await;
        }
    });
}

fn schedule_idle_gpu_shutdown(app: AppHandle, lifecycle: ComputeLifecycle, revision: u64) {
    let Some(idle_minutes) = lifecycle.snapshot().idle_shutdown_minutes else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(idle_minutes * 60)).await;
        if !lifecycle.is_current(revision) {
            return;
        }
        let service = app.state::<ServiceClient>();
        let provider = app.state::<CompShareProvider>();
        let compute = app.state::<ComputeControlStore>();
        let Ok(instances) = compute.list_instances() else {
            return;
        };
        for managed in instances {
            if managed.role == ComputeInstanceRole::UserManaged
                || !managed.platform_state.eq_ignore_ascii_case("running")
                || managed.running_mode != "gpu"
                || compute
                    .active_worker_lease_count(&managed.instance_id)
                    .map_or(true, |count| count > 0)
                || !lifecycle.is_current(revision)
            {
                continue;
            }
            let Ok(probe) = service.probe_for(&managed.instance_id).await else {
                continue;
            };
            if probe.queue_active > 0 || probe.queue_queued > 0 || !lifecycle.is_current(revision) {
                continue;
            }
            let locator = compute_instance_locator(&managed);
            let Ok(instance) = provider.describe_instance(locator.clone()).await else {
                continue;
            };
            if instance.state != CompSharePowerState::Running
                || instance.running_mode != CompShareRunningMode::Gpu
            {
                continue;
            }
            if provider.stop_instance_for(locator.clone()).await.is_err() {
                continue;
            }
            let Ok(stopped) = wait_for_instance_state_for(
                &provider,
                &locator,
                CompSharePowerState::Stopped,
                None,
                Duration::from_secs(120),
            )
            .await
            else {
                continue;
            };
            if !lifecycle.is_current(revision) {
                continue;
            }
            let Ok(refreshed) = compute.refresh_platform_instance(&stopped) else {
                continue;
            };
            let _ = provider.delete_stop_scheduler_for(locator.clone()).await;
            if refreshed.role == ComputeInstanceRole::Primary && stopped.support_without_gpu_start {
                let _ = provider
                    .start_instance_for(locator, CompShareStartMode::NoGpu)
                    .await;
            } else if refreshed.role == ComputeInstanceRole::Elastic {
                let queue = app.state::<JobQueueStorage>();
                let _ =
                    release_idle_elastic_instance(&provider, &compute, &queue, &refreshed).await;
            }
        }
    });
}

fn safe_artifact_filename(artifact_id: &str, original_filename: &str) -> Result<String, String> {
    let extension = std::path::Path::new(original_filename)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "mp4" | "mov" | "webm") {
        return Err("远端视频的文件格式不受支持。".to_owned());
    }
    let stem = safe_path_component(artifact_id)?;
    Ok(format!("{stem}.{extension}"))
}

fn generation_frame_profile(parameters: &serde_json::Value) -> Result<FrameProfile, String> {
    let aspect_ratio = parameters
        .get("aspectRatio")
        .and_then(serde_json::Value::as_str)
        .and_then(FrameAspectRatio::from_label)
        .ok_or_else(|| "生成任务没有保存有效的项目画幅，已停止保存候选。".to_owned())?;
    let profile = aspect_ratio.profile();
    let dimensions = [
        ("width", profile.work.width),
        ("height", profile.work.height),
        ("visibleWidth", profile.visible.width),
        ("visibleHeight", profile.visible.height),
        ("cropX", profile.crop_x),
        ("cropY", profile.crop_y),
    ];
    let matches_contract = dimensions.iter().all(|(key, expected)| {
        parameters.get(key).and_then(serde_json::Value::as_u64) == Some(u64::from(*expected))
    });
    if !matches_contract {
        return Err(format!(
            "生成任务保存的画幅参数与 {} 契约不一致，已停止保存候选。",
            profile.aspect_ratio.label()
        ));
    }
    Ok(profile)
}

fn safe_path_component(value: &str) -> Result<String, String> {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() || sanitized.len() > 128 || sanitized == "." || sanitized == ".." {
        return Err("远端任务返回的文件标识无效。".to_owned());
    }
    Ok(sanitized)
}

#[tauri::command]
fn get_frame_composition(
    storage: State<'_, FrameCompositionStorage>,
    input: GetFrameCompositionInput,
) -> Result<Option<FrameComposition>, FrameCompositionError> {
    storage.get(input)
}

fn restart_idle_gpu_shutdown(app: &AppHandle, lifecycle: &ComputeLifecycle) {
    let revision = lifecycle.invalidate_idle_shutdown();
    schedule_idle_gpu_shutdown(app.clone(), lifecycle.clone(), revision);
}

fn is_terminal_service_job(job: &ServiceJob) -> bool {
    matches!(
        job.status.as_str(),
        "completed" | "failed" | "cancelled" | "interrupted"
    )
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadCompletedImageInput {
    project_id: String,
    job_id: String,
}

#[tauri::command]
fn save_frame_composition(
    storage: State<'_, FrameCompositionStorage>,
    input: SaveFrameCompositionInput,
) -> Result<FrameComposition, FrameCompositionError> {
    storage.save(input)
}

#[tauri::command]
fn prepare_frame_derivative(
    storage: State<'_, FrameCompositionStorage>,
    input: PrepareFrameDerivativeInput,
) -> Result<FrameComposition, FrameCompositionError> {
    storage.prepare(input)
}

#[tauri::command]
fn list_assets(
    storage: State<'_, AssetStorage>,
    project_id: String,
) -> Result<Vec<AssetItem>, AssetError> {
    storage.list(&project_id)
}

#[tauri::command]
fn update_asset(
    storage: State<'_, AssetStorage>,
    input: UpdateAssetInput,
) -> Result<AssetItem, AssetError> {
    storage.update(input)
}

#[tauri::command]
async fn replace_asset_file(
    storage: State<'_, AssetStorage>,
    input: ReplaceAssetFileInput,
) -> Result<AssetItem, AssetError> {
    let storage = storage.inner().clone();
    tauri::async_runtime::spawn_blocking(move || storage.replace_file(input))
        .await
        .map_err(|error| AssetError {
            code: "asset_task_failed".to_owned(),
            message: format!("素材替换任务失败：{error}"),
        })?
}

#[tauri::command]
fn set_current_asset_version(
    storage: State<'_, AssetStorage>,
    input: SetCurrentAssetVersionInput,
) -> Result<AssetItem, AssetError> {
    storage.set_current_version(input)
}

#[tauri::command]
fn delete_asset(storage: State<'_, AssetStorage>, asset_id: String) -> Result<(), AssetError> {
    storage.delete(&asset_id)
}

#[tauri::command]
fn unlink_asset(
    storage: State<'_, AssetStorage>,
    input: UnlinkAssetInput,
) -> Result<AssetItem, AssetError> {
    storage.unlink(input)
}

#[tauri::command]
async fn import_asset_files(
    storage: State<'_, AssetStorage>,
    input: ImportAssetFilesInput,
) -> Result<Vec<AssetItem>, AssetError> {
    let storage = storage.inner().clone();
    tauri::async_runtime::spawn_blocking(move || storage.import_files(input))
        .await
        .map_err(|error| AssetError {
            code: "asset_task_failed".to_owned(),
            message: format!("素材导入任务失败：{error}"),
        })?
}

#[tauri::command]
async fn import_asset_payload(
    storage: State<'_, AssetStorage>,
    input: ImportAssetPayloadInput,
) -> Result<AssetItem, AssetError> {
    let storage = storage.inner().clone();
    tauri::async_runtime::spawn_blocking(move || storage.import_payload(input))
        .await
        .map_err(|error| AssetError {
            code: "asset_task_failed".to_owned(),
            message: format!("剪贴板素材导入失败：{error}"),
        })?
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoragePreferences {
    projects_root: Option<PathBuf>,
}

fn storage_preferences_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("storage-preferences.json"))
        .map_err(|error| format!("无法确定应用数据目录：{error}"))
}

fn load_storage_preferences(app: &AppHandle) -> Result<StoragePreferences, String> {
    let path = storage_preferences_path(app)?;
    if !path.exists() {
        return Ok(StoragePreferences::default());
    }
    let payload = fs::read(&path).map_err(|error| format!("无法读取存储设置：{error}"))?;
    serde_json::from_slice(&payload).map_err(|error| format!("存储设置格式无效：{error}"))
}

fn save_storage_preferences(
    app: &AppHandle,
    preferences: &StoragePreferences,
) -> Result<(), String> {
    let path = storage_preferences_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建应用数据目录：{error}"))?;
    }
    let payload = serde_json::to_vec_pretty(preferences)
        .map_err(|error| format!("无法保存存储设置：{error}"))?;
    fs::write(path, payload).map_err(|error| format!("无法保存存储设置：{error}"))
}

fn default_projects_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("projects"))
        .map_err(|error| format!("无法确定应用数据目录：{error}"))
}

fn storage_info_with_preferences(
    app: &AppHandle,
    storage: &ProjectStorage,
) -> Result<StorageInfo, String> {
    let default_root = default_projects_root(app)?;
    let configured_root = load_storage_preferences(app)?
        .projects_root
        .unwrap_or_else(|| default_root.clone());
    let mut info = storage.info();
    info.default_projects_root = default_root;
    info.configured_projects_root = configured_root.clone();
    info.restart_required = configured_root != info.projects_root;
    info.configured_root_available = configured_root.is_dir();
    Ok(info)
}

fn initialize_storage(app: &AppHandle) -> Result<ProjectStorage, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定应用数据目录：{error}"))?;
    let database_path = app_data_dir.join("zhihua.sqlite3");
    let default_root = app_data_dir.join("projects");
    if let Some(configured_root) = load_storage_preferences(app)?.projects_root {
        if configured_root.is_absolute() && configured_root.parent().is_some() {
            if let Ok(storage) = ProjectStorage::initialize(&database_path, configured_root) {
                return Ok(storage);
            }
        }
    }
    ProjectStorage::initialize(database_path, default_root).map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let storage = initialize_storage(app.handle())
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
            app.asset_protocol_scope()
                .allow_directory(&storage.info().projects_root, true)
                .map_err(|error| -> Box<dyn std::error::Error> {
                    format!("无法授权项目素材目录：{error}").into()
                })?;
            let source_storage = SourceStorage::initialize(storage.clone())
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
            let storyboard_storage = StoryboardStorage::initialize(storage.clone())
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
            let asset_storage = AssetStorage::initialize(storage.clone())
                .map_err(|error| -> Box<dyn std::error::Error> { error.message.into() })?;
            let frame_compositions = FrameCompositionStorage::initialize(storage.clone())
                .map_err(|error| -> Box<dyn std::error::Error> { error.message.into() })?;
            let generation_storage = GenerationStorage::initialize(storage.clone())
                .map_err(|error| -> Box<dyn std::error::Error> { error.message.into() })?;
            let job_queue = JobQueueStorage::initialize(storage.clone())
                .map_err(|error| -> Box<dyn std::error::Error> { error.message.into() })?;
            let tts = SystemTtsProvider::initialize(storage.clone())
                .map_err(|error| -> Box<dyn std::error::Error> { error.message.into() })?;
            let exporter = FfmpegExporter::new(storage.clone());
            let app_data_dir = app
                .handle()
                .path()
                .app_data_dir()
                .map_err(|error| format!("无法确定应用数据目录：{error}"))?;
            let database_path = storage.info().database_path;
            let llm = LlmProvider::initialize(app_data_dir.clone(), database_path.clone())
                .map_err(|error| -> Box<dyn std::error::Error> { error.message.into() })?;
            let service = ServiceClient::new(app_data_dir.clone())
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
            let comp_share = CompShareProvider::new(app_data_dir.clone())
                .map_err(|error| -> Box<dyn std::error::Error> { error.message.into() })?;
            let compute_control = ComputeControlStore::initialize(database_path)
                .map_err(|error| -> Box<dyn std::error::Error> { error.message.into() })?;
            let compute_lifecycle =
                ComputeLifecycle::load(app_data_dir.join("compute-policy.json"));
            let ssh_tunnel = SshTunnelManager::new(app_data_dir)
                .map_err(|error| -> Box<dyn std::error::Error> { error.message.into() })?;
            app.manage(storage);
            app.manage(source_storage);
            app.manage(storyboard_storage);
            app.manage(asset_storage);
            app.manage(frame_compositions);
            app.manage(generation_storage);
            app.manage(job_queue);
            app.manage(tts);
            app.manage(exporter);
            app.manage(llm);
            app.manage(compute_lifecycle);
            app.manage(compute_control);
            app.manage(service);
            app.manage(comp_share);
            app.manage(ssh_tunnel);
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let comp_share = app_handle.state::<CompShareProvider>();
                let compute_control = app_handle.state::<ComputeControlStore>();
                if let Ok(configuration) = comp_share.configuration() {
                    if configuration.credentials_stored {
                        if let Ok(instances) = comp_share
                            .list_instances(ListCompShareInstancesInput {
                                region: None,
                                zone: None,
                            })
                            .await
                        {
                            if let Ok(managed_instances) = compute_control
                                .reconcile(&instances, configuration.bound_instance_id.as_deref())
                            {
                                let queue = app_handle.state::<JobQueueStorage>();
                                for managed in managed_instances {
                                    let _ = release_idle_elastic_instance(
                                        &comp_share,
                                        &compute_control,
                                        &queue,
                                        &managed,
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                }
                let manager = app_handle.state::<SshTunnelManager>();
                let configured_instance = comp_share
                    .configuration()
                    .ok()
                    .and_then(|configuration| configuration.bound_instance_id)
                    .filter(|instance_id| {
                        manager
                            .status_for(instance_id)
                            .map(|status| status.configured)
                            .unwrap_or(false)
                    });
                if let Some(instance_id) = configured_instance {
                    let service = app_handle.state::<ServiceClient>();
                    let compute = app_handle.state::<ComputeControlStore>();
                    let connected = connect_service_through_tunnel_impl(
                        &manager,
                        &service,
                        &compute,
                        &instance_id,
                    )
                    .await;
                    if connected.is_ok() {
                        let _ = recover_generation_jobs(&app_handle).await;
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_storage_info,
            get_compute_policy,
            set_compute_policy,
            get_storage_usage,
            open_projects_root,
            set_projects_root,
            reset_projects_root,
            create_project,
            list_projects,
            update_project,
            rename_project,
            duplicate_project,
            delete_project,
            open_project,
            list_sources,
            import_source_file,
            create_pasted_source,
            read_source_text,
            set_source_enabled,
            delete_source,
            get_llm_configuration,
            save_llm_configuration,
            clear_llm_api_key,
            test_llm_connection,
            knowledge_point_list,
            knowledge_points_replace,
            knowledge_extract,
            storyboard_generate_from_knowledge,
            storyboard_revise_scene,
            list_storyboard_scenes,
            upsert_storyboard_scene,
            delete_storyboard_scene,
            reorder_storyboard_scenes,
            list_system_voices,
            get_scene_narration,
            synthesize_scene_narration,
            import_scene_narration,
            inspect_export_capability,
            export_project_video,
            preview_project_video,
            test_service_connection,
            get_service_connection_info,
            save_service_connection,
            clear_service_connection,
            probe_service,
            submit_service_job,
            get_service_job,
            cancel_service_job,
            list_local_jobs,
            plan_generation_compute_pool,
            reserve_service_worker,
            release_service_worker_reservation,
            upload_service_input,
            delete_service_input,
            download_service_artifact,
            apply_storyboard_edit,
            list_candidate_versions,
            list_enhanced_versions,
            inspect_candidate_audio,
            open_candidate_location,
            select_candidate_version,
            download_completed_job,
            download_completed_image_job,
            download_completed_enhancement,
            get_frame_composition,
            save_frame_composition,
            prepare_frame_derivative,
            list_assets,
            import_asset_files,
            import_asset_payload,
            update_asset,
            replace_asset_file,
            set_current_asset_version,
            delete_asset,
            unlink_asset,
            get_compshare_configuration,
            save_compshare_credentials,
            clear_compshare_credentials,
            test_compshare_connection,
            get_compshare_balance,
            list_compshare_instances,
            list_compshare_zones,
            preflight_compshare_create,
            list_managed_compute_instances,
            set_user_compute_worker_enabled,
            list_compute_worker_readiness,
            reconcile_compute_instances,
            get_compute_release_eligibility,
            create_managed_compshare_instance,
            release_managed_compshare_instance,
            bind_compshare_instance,
            get_bound_compshare_instance,
            start_compshare_instance,
            stop_compshare_instance,
            prepare_application_exit,
            update_compshare_stop_scheduler,
            delete_compshare_stop_scheduler,
            save_ssh_tunnel_configuration,
            start_ssh_tunnel,
            stop_ssh_tunnel,
            get_ssh_tunnel_status,
            connect_service_through_tunnel,
            connect_compute_worker,
            provision_compute_worker_connection,
            prepare_job_result_access,
            prepare_compute_worker,
            prepare_generation_service,
        ])
        .run(tauri::generate_context!())
        .expect("error while running zhihua");
}
