mod asset;
mod comp_share;
mod service;
mod source;
mod ssh_tunnel;
mod storage;
mod storyboard;

use asset::{AssetError, AssetItem, AssetStorage, ImportAssetFilesInput};
use comp_share::{
    BindCompShareInstanceInput, CompShareActionResult, CompShareBalance, CompShareConfiguration,
    CompShareConnectionTest, CompShareError, CompShareInstance, CompSharePowerState,
    CompShareProvider, CompShareRunningMode, CompShareSchedulerResult, CompShareStartMode,
    ListCompShareInstancesInput, SaveCompShareCredentialsInput, UpdateCompShareStopSchedulerInput,
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
use std::time::Duration;
use storage::{CreateProjectInput, Project, ProjectStorage, StorageInfo, UpdateProjectInput};
use storyboard::{ReorderScenesInput, SceneDraft, StoryboardStorage};
use tauri::{AppHandle, Manager, State};

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
) -> Result<ServiceProbe, String> {
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
    let _ = comp_share
        .update_stop_scheduler(UpdateCompShareStopSchedulerInput {
            stop_time: chrono::Utc::now().timestamp() + 60 * 60,
            project_id: None,
        })
        .await;

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
fn list_assets(
    storage: State<'_, AssetStorage>,
    project_id: String,
) -> Result<Vec<AssetItem>, AssetError> {
    storage.list(&project_id)
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
            list_assets,
            import_asset_files,
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
