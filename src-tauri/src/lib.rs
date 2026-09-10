mod asset;
mod comp_share;
mod generation;
mod service;
mod source;
mod ssh_tunnel;
mod storage;
mod storyboard;

use asset::{
    AssetError, AssetItem, AssetStorage, ImportAssetFilesInput, ImportAssetPayloadInput,
    ReplaceAssetFileInput, SetCurrentAssetVersionInput, UnlinkAssetInput, UpdateAssetInput,
};
use comp_share::{
    BindCompShareInstanceInput, CompShareActionResult, CompShareBalance, CompShareConfiguration,
    CompShareConnectionTest, CompShareError, CompShareInstance, CompSharePowerState,
    CompShareProvider, CompShareRunningMode, CompShareSchedulerResult, CompShareStartMode,
    ListCompShareInstancesInput, SaveCompShareCredentialsInput, UpdateCompShareStopSchedulerInput,
};
use generation::{CandidateVersion, GenerationError, GenerationStorage, RecordCandidateInput};
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
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use storage::{CreateProjectInput, Project, ProjectStorage, StorageInfo, UpdateProjectInput};
use storyboard::{ReorderScenesInput, SceneDraft, StoryboardStorage};
use tauri::{AppHandle, Manager, State};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadCompletedJobInput {
    project_id: String,
    job_id: String,
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

#[derive(Clone, Default)]
struct ComputeLifecycle {
    revision: Arc<AtomicU64>,
}

impl ComputeLifecycle {
    fn invalidate_idle_shutdown(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn is_current(&self, revision: u64) -> bool {
        self.revision.load(Ordering::SeqCst) == revision
    }
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
async fn bind_compshare_instance(
    provider: State<'_, CompShareProvider>,
    input: BindCompShareInstanceInput,
) -> Result<CompShareInstance, CompShareError> {
    provider.bind_instance(input).await
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
    mode: CompShareStartMode,
) -> Result<CompShareActionResult, CompShareError> {
    provider.start_instance(mode).await
}

#[tauri::command]
async fn stop_compshare_instance(
    provider: State<'_, CompShareProvider>,
) -> Result<CompShareActionResult, CompShareError> {
    provider.stop_instance().await
}

#[tauri::command]
async fn update_compshare_stop_scheduler(
    provider: State<'_, CompShareProvider>,
    input: UpdateCompShareStopSchedulerInput,
) -> Result<CompShareSchedulerResult, CompShareError> {
    provider.update_stop_scheduler(input).await
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
    wait_for_instance_state(
        &comp_share,
        CompSharePowerState::Running,
        Some(CompShareRunningMode::Gpu),
        Duration::from_secs(180),
    )
    .await?;
    if let Err(error) = comp_share
        .update_stop_scheduler(UpdateCompShareStopSchedulerInput {
            stop_time: chrono::Utc::now().timestamp() + 60 * 60,
            project_id: None,
        })
        .await
    {
        let _ = comp_share.stop_instance().await;
        return Err(format!(
            "GPU 已启动，但无法设置 60 分钟定时关机保障，已请求关机且任务未提交：{}",
            error.message
        ));
    }

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
    service: State<'_, ServiceClient>,
    input: SubmitServiceJobInput,
) -> Result<ServiceJob, ServiceConnectionError> {
    service.submit_job(input).await
}

#[tauri::command]
async fn get_service_job(
    service: State<'_, ServiceClient>,
    job_id: String,
) -> Result<ServiceJob, ServiceConnectionError> {
    service.get_job(&job_id).await
}

#[tauri::command]
async fn cancel_service_job(
    service: State<'_, ServiceClient>,
    job_id: String,
) -> Result<ServiceJob, ServiceConnectionError> {
    service.cancel_job(&job_id).await
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
fn list_candidate_versions(
    storage: State<'_, GenerationStorage>,
    input: CandidateVersionsInput,
) -> Result<Vec<CandidateVersion>, GenerationError> {
    storage.list(&input.project_id, &input.scene_id)
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

    let mut candidates = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        let filename = safe_artifact_filename(&artifact.artifact_id, &artifact.filename)?;
        let destination = project
            .project_dir
            .join("cache")
            .join("drafts")
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
    Ok(candidates)
}

fn schedule_idle_gpu_shutdown(app: AppHandle, lifecycle: ComputeLifecycle, revision: u64) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(180)).await;
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
            let _ = provider.stop_instance().await;
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
            let generation_storage = GenerationStorage::initialize(storage.clone())
                .map_err(|error| -> Box<dyn std::error::Error> { error.message.into() })?;
            let service = ServiceClient::new(
                app.handle()
                    .path()
                    .app_data_dir()
                    .map_err(|error| format!("无法确定应用数据目录：{error}"))?,
            )
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
            let comp_share = CompShareProvider::new(
                app.handle()
                    .path()
                    .app_data_dir()
                    .map_err(|error| format!("无法确定应用数据目录：{error}"))?,
            )
            .map_err(|error| -> Box<dyn std::error::Error> { error.message.into() })?;
            let ssh_tunnel = SshTunnelManager::new(
                app.handle()
                    .path()
                    .app_data_dir()
                    .map_err(|error| format!("无法确定应用数据目录：{error}"))?,
            )
            .map_err(|error| -> Box<dyn std::error::Error> { error.message.into() })?;
            app.manage(storage);
            app.manage(source_storage);
            app.manage(storyboard_storage);
            app.manage(asset_storage);
            app.manage(generation_storage);
            app.manage(ComputeLifecycle::default());
            app.manage(service);
            app.manage(comp_share);
            app.manage(ssh_tunnel);
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let manager = app_handle.state::<SshTunnelManager>();
                let configured = manager
                    .status()
                    .map(|status| status.configured)
                    .unwrap_or(false);
                if configured {
                    let service = app_handle.state::<ServiceClient>();
                    let comp_share = app_handle.state::<CompShareProvider>();
                    let _ =
                        connect_service_through_tunnel_impl(&manager, &service, &comp_share).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_storage_info,
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
            list_storyboard_scenes,
            upsert_storyboard_scene,
            delete_storyboard_scene,
            reorder_storyboard_scenes,
            test_service_connection,
            get_service_connection_info,
            save_service_connection,
            clear_service_connection,
            probe_service,
            submit_service_job,
            get_service_job,
            cancel_service_job,
            upload_service_input,
            delete_service_input,
            download_service_artifact,
            list_candidate_versions,
            select_candidate_version,
            download_completed_job,
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
            bind_compshare_instance,
            get_bound_compshare_instance,
            start_compshare_instance,
            stop_compshare_instance,
            update_compshare_stop_scheduler,
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
