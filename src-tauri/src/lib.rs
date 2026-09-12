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
    ComputeControlStore, ComputeInstanceRole, ComputeOperation, ComputeOperationAction,
    ComputeOperationStatus, ManagedComputeInstance, ReleaseEligibility,
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
    aspect_ratio: String,
    work_width: u32,
    work_height: u32,
    visible_width: u32,
    visible_height: u32,
    crop_x: u32,
    crop_y: u32,
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
    instance_id: String,
) -> Result<ReleaseEligibility, String> {
    store
        .release_eligibility(&instance_id)
        .map_err(|error| error.message)
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
    input: ReleaseManagedComputeInstanceInput,
) -> Result<ComputeOperation, String> {
    if !input.confirmed {
        return Err("释放实例需要用户明确确认".to_owned());
    }
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

    let eligibility = store
        .release_eligibility_excluding(&input.instance_id, Some(&input.idempotency_key))
        .map_err(|error| error.message)?;
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
    lifecycle: State<'_, ComputeLifecycle>,
) -> Result<ApplicationExitProtection, String> {
    lifecycle.invalidate_idle_shutdown();
    let keep_alive_policy = lifecycle.policy();

    let instance = match provider.bound_instance().await {
        Ok(instance) => instance,
        Err(error) => {
            return Ok(ApplicationExitProtection {
                active_remote_tasks: 0,
                action: "no-bound-instance".to_owned(),
                detail: format!("没有可处理的运行实例：{}", error.message),
            });
        }
    };
    if instance.state != CompSharePowerState::Running
        || instance.running_mode != CompShareRunningMode::Gpu
    {
        return Ok(ApplicationExitProtection {
            active_remote_tasks: 0,
            action: "no-gpu-cost".to_owned(),
            detail: "当前没有运行中的 GPU，无需额外处理。".to_owned(),
        });
    }

    if keep_alive_policy == ComputeKeepAlivePolicy::Continuous {
        if instance.stop_scheduler_time.is_none() {
            let snapshot = lifecycle.snapshot();
            provider
                .update_stop_scheduler(UpdateCompShareStopSchedulerInput {
                    stop_time: chrono::Utc::now().timestamp()
                        + (snapshot.hard_limit_minutes as i64) * 60,
                    project_id: instance.project_id.clone(),
                })
                .await
                .map_err(|error| format!("持续 GPU 尚未获得平台关机保障：{}", error.message))?;
        }
        return Ok(ApplicationExitProtection {
            active_remote_tasks: 0,
            action: "continuous-gpu".to_owned(),
            detail: "持续 GPU 模式已启用；客户端退出后保持 GPU，并由 12 小时平台硬上限兜底。"
                .to_owned(),
        });
    }

    // The service queue is authoritative. Local jobs may be stale after a tunnel
    // interruption, so an unreachable service must keep the platform watchdog.
    let probe = match service.probe().await {
        Ok(probe) => probe,
        Err(error) => {
            return Ok(ApplicationExitProtection {
                active_remote_tasks: 0,
                action: "platform-watchdog".to_owned(),
                detail: format!(
                    "无法确认远端队列（{}）；保留优云智算平台定时关机，避免误停仍在执行的任务。",
                    error.message
                ),
            });
        }
    };
    let active_remote_tasks = probe.queue_active.saturating_add(probe.queue_queued);
    if active_remote_tasks > 0 {
        if instance.stop_scheduler_time.is_none() {
            let snapshot = lifecycle.snapshot();
            provider
                .update_stop_scheduler(UpdateCompShareStopSchedulerInput {
                    stop_time: chrono::Utc::now().timestamp()
                        + (snapshot.hard_limit_minutes as i64) * 60,
                    project_id: instance.project_id.clone(),
                })
                .await
                .map_err(|error| {
                    format!(
                        "远端任务仍在运行，但无法补设平台关机保障：{}",
                        error.message
                    )
                })?;
        }
        return Ok(ApplicationExitProtection {
            active_remote_tasks,
            action: "continue-until-watchdog".to_owned(),
            detail: format!(
                "远端仍有 {active_remote_tasks} 个任务，退出后继续执行，并由平台运行上限定时关机兜底。"
            ),
        });
    }

    provider
        .stop_instance()
        .await
        .map_err(|error| format!("远端队列已空，但请求关闭 GPU 失败：{}", error.message))?;
    // A new GPU start always overwrites the safeguard. Deleting it here avoids
    // an old deadline unexpectedly affecting a later no-GPU maintenance boot.
    let _ = provider.delete_stop_scheduler().await;
    Ok(ApplicationExitProtection {
        active_remote_tasks: 0,
        action: "gpu-stop-requested".to_owned(),
        detail: "远端队列为空，已在客户端退出前请求关闭 GPU。".to_owned(),
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
) -> Result<TunnelStatus, TunnelError> {
    manager.start().await
}

#[tauri::command]
async fn stop_ssh_tunnel(
    manager: State<'_, SshTunnelManager>,
) -> Result<TunnelStatus, TunnelError> {
    manager.stop().await
}

#[tauri::command]
fn get_ssh_tunnel_status(
    manager: State<'_, SshTunnelManager>,
) -> Result<TunnelStatus, TunnelError> {
    manager.status()
}

async fn connect_service_through_tunnel_impl(
    manager: &SshTunnelManager,
    service: &ServiceClient,
    comp_share: &CompShareProvider,
) -> Result<ServiceProbe, String> {
    let tunnel = manager.start().await.map_err(|error| error.message)?;
    let local_url = tunnel
        .local_url
        .ok_or_else(|| "SSH 隧道没有返回本机服务地址".to_owned())?;
    let connection_info = service.info().map_err(|error| error.message)?;
    if connection_info.configured && connection_info.credential_stored {
        service.retarget(local_url).map_err(|error| error.message)?;
    } else {
        let instance_id = comp_share
            .configuration()
            .map_err(|error| error.message)?
            .bound_instance_id
            .ok_or_else(|| "尚未绑定优云智算实例".to_owned())?;
        let token = manager
            .read_service_token()
            .await
            .map_err(|error| error.message)?;
        service
            .save(SaveServiceConnectionInput {
                instance_id,
                base_url: local_url,
                token,
            })
            .map_err(|error| error.message)?;
    }
    service.probe().await.map_err(|error| error.message)
}

#[tauri::command]
async fn connect_service_through_tunnel(
    manager: State<'_, SshTunnelManager>,
    service: State<'_, ServiceClient>,
    comp_share: State<'_, CompShareProvider>,
) -> Result<ServiceProbe, String> {
    connect_service_through_tunnel_impl(&manager, &service, &comp_share).await
}

#[tauri::command]
async fn prepare_generation_service(
    manager: State<'_, SshTunnelManager>,
    service: State<'_, ServiceClient>,
    comp_share: State<'_, CompShareProvider>,
    lifecycle: State<'_, ComputeLifecycle>,
) -> Result<ServiceProbe, String> {
    lifecycle.invalidate_idle_shutdown();
    let mut instance = comp_share
        .bound_instance()
        .await
        .map_err(|error| error.message)?;
    if instance.running_mode == CompShareRunningMode::NoGpu {
        comp_share
            .stop_instance()
            .await
            .map_err(|error| error.message)?;
        instance = wait_for_instance_state(
            &comp_share,
            CompSharePowerState::Stopped,
            None,
            Duration::from_secs(120),
        )
        .await?;
    }
    if instance.state == CompSharePowerState::Stopped {
        comp_share
            .start_instance(CompShareStartMode::Gpu)
            .await
            .map_err(|error| error.message)?;
    }
    let hard_limit_minutes = lifecycle.snapshot().hard_limit_minutes;
    if let Err(error) = comp_share
        .update_stop_scheduler(UpdateCompShareStopSchedulerInput {
            stop_time: chrono::Utc::now().timestamp() + (hard_limit_minutes as i64) * 60,
            project_id: None,
        })
        .await
    {
        let _ = wait_for_instance_state(
            &comp_share,
            CompSharePowerState::Running,
            Some(CompShareRunningMode::Gpu),
            Duration::from_secs(180),
        )
        .await;
        let _ = comp_share.stop_instance().await;
        return Err(format!(
            "GPU 已启动，但无法设置 {hard_limit_minutes} 分钟定时关机保障，已请求关机且任务未提交：{}",
            error.message
        ));
    }
    wait_for_instance_state(
        &comp_share,
        CompSharePowerState::Running,
        Some(CompShareRunningMode::Gpu),
        Duration::from_secs(180),
    )
    .await?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(240);
    loop {
        let readiness =
            match connect_service_through_tunnel_impl(&manager, &service, &comp_share).await {
                Ok(probe) if probe.comfyui_ready => return Ok(probe),
                Ok(probe) => probe.detail,
                Err(error) => error,
            };
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "GPU 已启动，但生成环境未在 4 分钟内就绪：{readiness}"
            ));
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

async fn wait_for_instance_state(
    provider: &CompShareProvider,
    expected_state: CompSharePowerState,
    expected_mode: Option<CompShareRunningMode>,
    timeout: Duration,
) -> Result<CompShareInstance, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let instance = provider
            .bound_instance()
            .await
            .map_err(|error| error.message)?;
        if instance.state == expected_state
            && expected_mode.map_or(true, |mode| instance.running_mode == mode)
        {
            return Ok(instance);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "实例状态切换超时，平台当前返回 {}",
                instance.raw_state
            ));
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

#[tauri::command]
fn get_storage_info(storage: State<'_, ProjectStorage>) -> StorageInfo {
    storage.info()
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
    lifecycle: State<'_, ComputeLifecycle>,
    input: SubmitServiceJobInput,
) -> Result<ServiceJob, ServiceConnectionError> {
    let request_id = input.client_request_id.clone();
    let result = async {
        queue.stage(&input).map_err(queue_service_error)?;
        queue
            .claim_for_worker(&request_id, "primary-worker", 300)
            .map_err(queue_service_error)?;
        match service.submit_job(input).await {
            Ok(job) => {
                queue.record_remote(&job).map_err(queue_service_error)?;
                Ok(job)
            }
            Err(error) => {
                let _ = queue.record_submit_failure(&request_id, error.code, &error.message);
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
    lifecycle: State<'_, ComputeLifecycle>,
    job_id: String,
) -> Result<ServiceJob, ServiceConnectionError> {
    let job = service.get_job(&job_id).await?;
    queue.sync(&job).map_err(queue_service_error)?;
    if is_terminal_service_job(&job) {
        restart_idle_gpu_shutdown(&app, &lifecycle);
    }
    Ok(job)
}

#[tauri::command]
async fn cancel_service_job(
    app: AppHandle,
    service: State<'_, ServiceClient>,
    queue: State<'_, JobQueueStorage>,
    lifecycle: State<'_, ComputeLifecycle>,
    job_id: String,
) -> Result<ServiceJob, ServiceConnectionError> {
    let job = service.cancel_job(&job_id).await?;
    queue.sync(&job).map_err(queue_service_error)?;
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

#[tauri::command]
fn plan_generation_compute_pool(input: ComputePoolPlanInput) -> ComputePoolPlan {
    plan_compute_pool(input)
}

#[tauri::command]
async fn upload_service_input(
    service: State<'_, ServiceClient>,
    source_path: String,
) -> Result<ServiceInputUpload, ServiceConnectionError> {
    service.upload_input(&source_path).await
}

#[tauri::command]
async fn delete_service_input(
    service: State<'_, ServiceClient>,
    input_id: String,
) -> Result<(), ServiceConnectionError> {
    service.delete_input(&input_id).await
}

#[tauri::command]
async fn download_service_artifact(
    service: State<'_, ServiceClient>,
    input: DownloadServiceArtifactInput,
) -> Result<ServiceArtifactDownload, ServiceConnectionError> {
    service.download_artifact(input).await
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

#[tauri::command]
async fn download_completed_job(
    app: AppHandle,
    projects: State<'_, ProjectStorage>,
    storyboards: State<'_, StoryboardStorage>,
    generations: State<'_, GenerationStorage>,
    queue: State<'_, JobQueueStorage>,
    service: State<'_, ServiceClient>,
    lifecycle: State<'_, ComputeLifecycle>,
    input: DownloadCompletedJobInput,
) -> Result<Vec<CandidateVersion>, String> {
    let project = projects
        .get_project(&input.project_id)
        .map_err(|error| error.to_string())?;
    let job = service
        .get_job(&input.job_id)
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
        let downloaded = service
            .download_artifact(DownloadServiceArtifactInput {
                job_id: job.id.clone(),
                artifact_id: artifact.artifact_id.clone(),
                destination_path: destination.to_string_lossy().into_owned(),
                expected_size_bytes: artifact.size_bytes,
                expected_sha256: artifact.sha256.clone(),
            })
            .await
            .map_err(|error| error.message)?;
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
                    aspect_ratio: input.aspect_ratio.clone(),
                    work_width: input.work_width,
                    work_height: input.work_height,
                    visible_width: input.visible_width,
                    visible_height: input.visible_height,
                    crop_x: input.crop_x,
                    crop_y: input.crop_y,
                })
                .map_err(|error| error.message)?,
        );
    }
    let revision = lifecycle.invalidate_idle_shutdown();
    schedule_idle_gpu_shutdown(app, lifecycle.inner().clone(), revision);
    queue
        .mark_local_complete(&job.id)
        .map_err(|error| error.message)?;
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
    service: State<'_, ServiceClient>,
    lifecycle: State<'_, ComputeLifecycle>,
    input: DownloadCompletedImageInput,
) -> Result<Vec<AssetItem>, String> {
    let project = projects
        .get_project(&input.project_id)
        .map_err(|error| error.to_string())?;
    let job = service
        .get_job(&input.job_id)
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

    let mut imported = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        let filename = safe_artifact_filename(&artifact.artifact_id, &artifact.filename)?;
        let temporary = project
            .project_dir
            .join("cache")
            .join("image-jobs")
            .join(safe_path_component(&job.id)?)
            .join(filename);
        let downloaded = service
            .download_artifact(DownloadServiceArtifactInput {
                job_id: job.id.clone(),
                artifact_id: artifact.artifact_id,
                destination_path: temporary.to_string_lossy().into_owned(),
                expected_size_bytes: artifact.size_bytes,
                expected_sha256: artifact.sha256,
            })
            .await
            .map_err(|error| error.message)?;
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
    queue
        .mark_local_complete(&job.id)
        .map_err(|error| error.message)?;
    let revision = lifecycle.invalidate_idle_shutdown();
    schedule_idle_gpu_shutdown(app, lifecycle.inner().clone(), revision);
    Ok(imported)
}

#[tauri::command]
async fn download_completed_enhancement(
    app: AppHandle,
    projects: State<'_, ProjectStorage>,
    storyboards: State<'_, StoryboardStorage>,
    generations: State<'_, GenerationStorage>,
    queue: State<'_, JobQueueStorage>,
    service: State<'_, ServiceClient>,
    lifecycle: State<'_, ComputeLifecycle>,
    input: DownloadCompletedUpscaleInput,
) -> Result<Vec<EnhancedVersion>, String> {
    let project = projects
        .get_project(&input.project_id)
        .map_err(|error| error.to_string())?;
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
        .get_job(&input.job_id)
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
        let downloaded = service
            .download_artifact(DownloadServiceArtifactInput {
                job_id: job.id.clone(),
                artifact_id: artifact.artifact_id.clone(),
                destination_path: destination.to_string_lossy().into_owned(),
                expected_size_bytes: artifact.size_bytes,
                expected_sha256: artifact.sha256.clone(),
            })
            .await
            .map_err(|error| error.message)?;
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
    let revision = lifecycle.invalidate_idle_shutdown();
    schedule_idle_gpu_shutdown(app, lifecycle.inner().clone(), revision);
    queue
        .mark_local_complete(&job.id)
        .map_err(|error| error.message)?;
    Ok(enhanced_versions)
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
        let Ok(probe) = service.probe().await else {
            return;
        };
        if probe.queue_active > 0 || probe.queue_queued > 0 || !lifecycle.is_current(revision) {
            return;
        }
        let provider = app.state::<CompShareProvider>();
        let Ok(instance) = provider.bound_instance().await else {
            return;
        };
        if instance.state == CompSharePowerState::Running
            && instance.running_mode == CompShareRunningMode::Gpu
            && lifecycle.is_current(revision)
        {
            if provider.stop_instance().await.is_ok()
                && wait_for_instance_state(
                    &provider,
                    CompSharePowerState::Stopped,
                    None,
                    Duration::from_secs(120),
                )
                .await
                .is_ok()
                && lifecycle.is_current(revision)
            {
                let _ = provider.delete_stop_scheduler().await;
                let _ = provider.start_instance(CompShareStartMode::NoGpu).await;
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

fn initialize_storage(app: &AppHandle) -> Result<ProjectStorage, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定应用数据目录：{error}"))?;
    ProjectStorage::initialize(
        app_data_dir.join("zhihua.sqlite3"),
        app_data_dir.join("projects"),
    )
    .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let storage = initialize_storage(app.handle())
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
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
                            let _ = compute_control
                                .reconcile(&instances, configuration.bound_instance_id.as_deref());
                        }
                    }
                }
                let manager = app_handle.state::<SshTunnelManager>();
                let configured = manager
                    .status()
                    .map(|status| status.configured)
                    .unwrap_or(false);
                if configured {
                    let service = app_handle.state::<ServiceClient>();
                    let _ =
                        connect_service_through_tunnel_impl(&manager, &service, &comp_share).await;
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
            prepare_generation_service,
        ])
        .run(tauri::generate_context!())
        .expect("error while running zhihua");
}
