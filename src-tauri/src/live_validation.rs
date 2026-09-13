use crate::comp_share::{
    BindCompShareInstanceInput, CompShareInstanceLocator, CompSharePowerState, CompShareProvider,
    CompShareRunningMode, CompShareStartMode, ListCompShareInstancesInput,
    SaveCompShareCredentialsInput,
};
use crate::service::{
    DownloadServiceArtifactInput, SaveServiceConnectionInput, ServiceClient, ServiceJob,
    SubmitServiceJobInput,
};
use crate::ssh_tunnel::{SaveTunnelConfigurationInput, SshTunnelManager};
use serde_json::json;
use std::path::PathBuf;

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name}"))
}

async fn wait_for_job(service: &ServiceClient, id: &str, max_minutes: u64) -> ServiceJob {
    for _ in 0..(max_minutes * 20) {
        let job = service.get_job(id).await.expect("poll remote job");
        if job.status == "completed" {
            return job;
        }
        assert!(
            !matches!(job.status.as_str(), "failed" | "cancelled"),
            "remote job ended with {}: {:?}",
            job.status,
            job.error_message
        );
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
    panic!("remote job did not finish within {max_minutes} minutes");
}

#[tokio::test]
#[ignore = "requires an authorized running instance and updates the real desktop connection configuration"]
async fn configure_desktop_for_running_instance() {
    let app_data_dir = PathBuf::from(required("ZHIHUA_TEST_APP_DATA_DIR"));
    let instance_id = required("ZHIHUA_TEST_COMPSHARE_INSTANCE_ID");
    let private_key = std::fs::read_to_string(required("ZHIHUA_TEST_SSH_PRIVATE_KEY_PATH"))
        .expect("read local SSH private key");

    let compute =
        CompShareProvider::new(app_data_dir.clone()).expect("initialize CompShare provider");
    compute
        .save_credentials(SaveCompShareCredentialsInput {
            public_key: required("ZHIHUA_TEST_COMPSHARE_PUBLIC_KEY"),
            private_key: required("ZHIHUA_TEST_COMPSHARE_PRIVATE_KEY"),
        })
        .expect("save authorized CompShare credentials");
    let instance = compute
        .bind_instance(BindCompShareInstanceInput {
            instance_id: instance_id.clone(),
            region: required("ZHIHUA_TEST_COMPSHARE_REGION"),
            zone: required("ZHIHUA_TEST_COMPSHARE_ZONE"),
            project_id: None,
        })
        .await
        .expect("bind running instance");
    assert_eq!(instance.state, CompSharePowerState::Running);
    assert_eq!(instance.running_mode, CompShareRunningMode::Gpu);

    let tunnel = SshTunnelManager::new(app_data_dir.clone()).expect("initialize SSH tunnel");
    tunnel
        .save_configuration(SaveTunnelConfigurationInput {
            instance_id: instance_id.clone(),
            host: required("ZHIHUA_TEST_SSH_HOST"),
            port: required("ZHIHUA_TEST_SSH_PORT")
                .parse::<u16>()
                .expect("valid SSH port"),
            username: "root".to_owned(),
            host_key_fingerprint: required("ZHIHUA_TEST_SSH_HOST_FINGERPRINT"),
            private_key,
        })
        .expect("save SSH tunnel configuration");
    let tunnel_status = tunnel
        .start_for(&instance_id)
        .await
        .expect("start SSH tunnel");
    let local_url = tunnel_status.local_url.expect("local tunnel URL");
    let token = tunnel
        .read_service_token_for(&instance_id)
        .await
        .expect("read remote service token");

    let service = ServiceClient::new(app_data_dir).expect("initialize service client");
    service.clear().expect("clear stale service connection");
    service
        .save(SaveServiceConnectionInput {
            instance_id: instance_id.clone(),
            base_url: local_url,
            token,
        })
        .expect("save service connection");
    let probe = service.probe().await.expect("probe service through tunnel");
    assert!(probe.compatible);
    assert!(probe.comfyui_ready);
    assert!(probe.available_workflows.len() >= 11);
    tunnel
        .stop_for(&instance_id)
        .await
        .expect("stop validation tunnel");
}

#[tokio::test]
#[ignore = "requires an authorized running instance and adds its SSH profile to the real desktop worker pool"]
async fn configure_desktop_worker_connection() {
    let app_data_dir = PathBuf::from(required("ZHIHUA_TEST_APP_DATA_DIR"));
    let instance_id = required("ZHIHUA_TEST_COMPSHARE_INSTANCE_ID");
    let private_key = std::fs::read_to_string(required("ZHIHUA_TEST_SSH_PRIVATE_KEY_PATH"))
        .expect("read local SSH private key");
    let tunnel = SshTunnelManager::new(app_data_dir.clone()).expect("initialize SSH tunnel");
    tunnel
        .save_configuration(SaveTunnelConfigurationInput {
            instance_id: instance_id.clone(),
            host: required("ZHIHUA_TEST_SSH_HOST"),
            port: required("ZHIHUA_TEST_SSH_PORT")
                .parse::<u16>()
                .expect("valid SSH port"),
            username: "root".to_owned(),
            host_key_fingerprint: required("ZHIHUA_TEST_SSH_HOST_FINGERPRINT"),
            private_key,
        })
        .expect("save worker SSH tunnel configuration");
    let tunnel_status = tunnel
        .start_for(&instance_id)
        .await
        .expect("start worker SSH tunnel");
    let local_url = tunnel_status.local_url.expect("local tunnel URL");
    let token = tunnel
        .read_service_token_for(&instance_id)
        .await
        .expect("read remote service token");
    let service = ServiceClient::new(app_data_dir.clone()).expect("initialize service client");
    service
        .save_for(SaveServiceConnectionInput {
            instance_id: instance_id.clone(),
            base_url: local_url,
            token,
        })
        .expect("save worker service connection");
    let probe = service
        .probe_for(&instance_id)
        .await
        .expect("probe worker service through tunnel");
    assert!(probe.compatible);
    assert!(probe.workflows.len() >= 11);

    let compute = crate::compute_control::ComputeControlStore::initialize(
        app_data_dir.join("zhihua.sqlite3"),
    )
    .expect("initialize compute control store");
    let worker = compute
        .set_user_instance_worker_enabled(&instance_id, true)
        .expect("authorize existing instance for automatic worker control");
    assert_eq!(
        worker.role,
        crate::compute_control::ComputeInstanceRole::Elastic
    );
    assert_eq!(
        worker.ownership,
        crate::compute_control::ComputeInstanceOwnership::UserManaged
    );
    assert!(
        !compute
            .release_eligibility(&instance_id)
            .expect("read release eligibility")
            .allowed
    );
    tunnel
        .stop_for(&instance_id)
        .await
        .expect("stop validation tunnel");
}

#[tokio::test]
#[ignore = "starts one authorized instance without GPU, provisions its SSH key, probes the service, and stops it"]
async fn provision_desktop_worker_connection_without_gpu() {
    let app_data_dir = PathBuf::from(required("ZHIHUA_TEST_APP_DATA_DIR"));
    let instance_id = required("ZHIHUA_TEST_COMPSHARE_INSTANCE_ID");
    let provider = CompShareProvider::new(app_data_dir.clone()).expect("initialize CompShare");
    let platform = provider
        .list_instances(ListCompShareInstancesInput {
            region: None,
            zone: None,
        })
        .await
        .expect("list instances")
        .into_iter()
        .find(|instance| instance.instance_id == instance_id)
        .expect("target instance exists");
    let locator = CompShareInstanceLocator {
        instance_id: instance_id.clone(),
        region: platform.region,
        zone: platform.zone,
        project_id: platform.project_id,
    };
    assert!(
        platform.support_without_gpu_start,
        "instance supports no-GPU start"
    );
    if platform.state != CompSharePowerState::Stopped {
        provider
            .stop_instance_for(locator.clone())
            .await
            .expect("stop target before validation");
        crate::wait_for_instance_state_for(
            &provider,
            &locator,
            CompSharePowerState::Stopped,
            None,
            std::time::Duration::from_secs(180),
        )
        .await
        .expect("wait for stopped target");
    }

    let tunnel = SshTunnelManager::new(app_data_dir.clone()).expect("initialize SSH tunnel");
    let service = ServiceClient::new(app_data_dir).expect("initialize service client");
    let validation = async {
        provider
            .start_instance_for(locator.clone(), CompShareStartMode::NoGpu)
            .await
            .map_err(|error| error.message)?;
        provider
            .update_stop_scheduler_for(locator.clone(), chrono::Utc::now().timestamp() + 10 * 60)
            .await
            .map_err(|error| error.message)?;
        let running = crate::wait_for_instance_state_for(
            &provider,
            &locator,
            CompSharePowerState::Running,
            Some(CompShareRunningMode::NoGpu),
            std::time::Duration::from_secs(180),
        )
        .await?;
        if running.gpu_count != Some(0) {
            return Err(format!(
                "instance unexpectedly reported {:?} GPUs in no-GPU validation",
                running.gpu_count
            ));
        }

        let access = provider
            .ssh_access_for(locator.clone())
            .await
            .map_err(|error| error.message)?;
        tunnel
            .bootstrap_configuration(
                &instance_id,
                &access.host,
                access.port,
                &access.username,
                &access.password,
            )
            .await
            .map_err(|error| error.message)?;
        tunnel
            .ensure_remote_service_for(&instance_id)
            .await
            .map_err(|error| error.message)?;
        let status = tunnel
            .start_for(&instance_id)
            .await
            .map_err(|error| error.message)?;
        let token = tunnel
            .read_service_token_for(&instance_id)
            .await
            .map_err(|error| error.message)?;
        service
            .save_for(SaveServiceConnectionInput {
                instance_id: instance_id.clone(),
                base_url: status.local_url.expect("local tunnel URL"),
                token,
            })
            .map_err(|error| error.message)?;
        let probe = service
            .probe_for(&instance_id)
            .await
            .map_err(|error| error.message)?;
        if !probe.compatible {
            return Err("service API is incompatible".to_owned());
        }
        Ok::<_, String>(probe)
    }
    .await;

    let cleanup = async {
        let _ = tunnel.stop_for(&instance_id).await;
        provider
            .stop_instance_for(locator.clone())
            .await
            .map_err(|error| error.message)?;
        crate::wait_for_instance_state_for(
            &provider,
            &locator,
            CompSharePowerState::Stopped,
            None,
            std::time::Duration::from_secs(180),
        )
        .await
        .map(|_| ())
    }
    .await;
    if let Err(error) = cleanup {
        panic!(
            "failed to stop target after validation: {error}; validation result: {:?}",
            validation.as_ref().err()
        );
    }
    let probe = validation.expect("automatic no-GPU SSH provisioning succeeds");
    assert!(
        !probe.comfyui_ready,
        "no-GPU mode does not report generation ready"
    );
}

#[tokio::test]
#[ignore = "starts two configured desktop workers in paid GPU mode, validates pool fallback or distribution, and stops both"]
async fn prepare_desktop_worker_pool_and_cleanup() {
    let app_data_dir = PathBuf::from(required("ZHIHUA_TEST_APP_DATA_DIR"));
    let instance_ids = required("ZHIHUA_TEST_COMPSHARE_INSTANCE_IDS")
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(instance_ids.len(), 2, "provide exactly two instance IDs");

    let tunnel = SshTunnelManager::new(app_data_dir.clone()).expect("initialize SSH tunnel");
    let service = ServiceClient::new(app_data_dir.clone()).expect("initialize service client");
    let provider = CompShareProvider::new(app_data_dir.clone()).expect("initialize CompShare");
    let compute = crate::compute_control::ComputeControlStore::initialize(
        app_data_dir.join("zhihua.sqlite3"),
    )
    .expect("initialize compute control store");
    let policy_dir = tempfile::tempdir().expect("temporary policy directory");
    let lifecycle = crate::ComputeLifecycle::load(policy_dir.path().join("compute-policy.json"));

    let first = crate::prepare_compute_worker_impl(
        &tunnel,
        &service,
        &provider,
        &compute,
        &lifecycle,
        &instance_ids[0],
    );
    let second = crate::prepare_compute_worker_impl(
        &tunnel,
        &service,
        &provider,
        &compute,
        &lifecycle,
        &instance_ids[1],
    );
    let (first_result, second_result) = tokio::join!(first, second);

    let mut validation_error = None;
    match (&first_result, &second_result) {
        (Ok(first_probe), Ok(second_probe)) => {
            if !first_probe.comfyui_ready || !second_probe.comfyui_ready {
                validation_error =
                    Some("both workers connected but ComfyUI was not ready".to_owned());
            } else {
                let first_lease = compute.acquire_ready_worker_lease("live-pool-job-1", 120);
                let second_lease = compute.acquire_ready_worker_lease("live-pool-job-2", 120);
                match (first_lease, second_lease) {
                    (Ok(first_lease), Ok(second_lease)) => {
                        if first_lease.instance_id == second_lease.instance_id {
                            validation_error = Some(
                                "two jobs were assigned to the same worker despite two ready workers"
                                    .to_owned(),
                            );
                        }
                    }
                    (Err(error), _) | (_, Err(error)) => {
                        validation_error = Some(format!("worker lease failed: {}", error.message));
                    }
                }
                let _ = compute.release_worker_lease("live-pool-job-1");
                let _ = compute.release_worker_lease("live-pool-job-2");
            }
        }
        (Ok(probe), Err(error)) | (Err(error), Ok(probe)) if probe.comfyui_ready => {
            let first_lease = compute.acquire_ready_worker_lease("live-pool-job-1", 120);
            let second_lease = compute.acquire_ready_worker_lease("live-pool-job-2", 120);
            match (first_lease, second_lease) {
                (Ok(first_lease), Ok(second_lease))
                    if first_lease.instance_id == second_lease.instance_id =>
                {
                    eprintln!(
                        "worker pool safely degraded to one route because the second worker was unavailable: {error}"
                    );
                }
                (Ok(_), Ok(_)) => {
                    validation_error = Some(
                        "degraded pool unexpectedly routed jobs to different workers".to_owned(),
                    );
                }
                (Err(error), _) | (_, Err(error)) => {
                    validation_error =
                        Some(format!("fallback worker lease failed: {}", error.message));
                }
            }
            let _ = compute.release_worker_lease("live-pool-job-1");
            let _ = compute.release_worker_lease("live-pool-job-2");
        }
        _ => {
            validation_error = Some(format!(
                "no usable worker after preparation: first={:?}; second={:?}",
                first_result.as_ref().err(),
                second_result.as_ref().err()
            ));
        }
    }

    let mut cleanup_errors = Vec::new();
    for instance_id in &instance_ids {
        let _ = tunnel.stop_for(instance_id).await;
        let managed = match compute.get_instance(instance_id) {
            Ok(value) => value,
            Err(error) => {
                cleanup_errors.push(format!("{instance_id}: {}", error.message));
                continue;
            }
        };
        let locator = crate::compute_instance_locator(&managed);
        let platform = provider.describe_instance(locator.clone()).await;
        if let Ok(platform) = platform {
            if platform.state == CompSharePowerState::Running
                || platform.state == CompSharePowerState::Starting
            {
                if let Err(error) = provider.stop_instance_for(locator.clone()).await {
                    cleanup_errors.push(format!("{instance_id}: {}", error.message));
                    continue;
                }
                match crate::wait_for_instance_state_for(
                    &provider,
                    &locator,
                    CompSharePowerState::Stopped,
                    None,
                    std::time::Duration::from_secs(180),
                )
                .await
                {
                    Ok(stopped) => {
                        let _ = compute.refresh_platform_instance(&stopped);
                    }
                    Err(error) => {
                        cleanup_errors.push(format!("{instance_id}: {error}"));
                        continue;
                    }
                }
            }
            if let Err(error) = provider.delete_stop_scheduler_for(locator.clone()).await {
                cleanup_errors.push(format!("{instance_id}: {}", error.message));
            } else if let Ok(refreshed) = provider.describe_instance(locator).await {
                let _ = compute.refresh_platform_instance(&refreshed);
            }
        }
    }
    assert!(
        cleanup_errors.is_empty(),
        "worker cleanup failed: {cleanup_errors:?}"
    );
    assert!(validation_error.is_none(), "{}", validation_error.unwrap());
}

#[tokio::test]
#[ignore = "requires saved CompShare credentials and reconciles the real desktop instance inventory"]
async fn reconcile_desktop_compute_inventory() {
    let app_data_dir = PathBuf::from(required("ZHIHUA_TEST_APP_DATA_DIR"));
    let provider = CompShareProvider::new(app_data_dir.clone()).expect("initialize CompShare");
    let instances = provider
        .list_instances(crate::comp_share::ListCompShareInstancesInput {
            region: None,
            zone: None,
        })
        .await
        .expect("list platform instances");
    let configuration = provider
        .configuration()
        .expect("read CompShare configuration");
    let compute = crate::compute_control::ComputeControlStore::initialize(
        app_data_dir.join("zhihua.sqlite3"),
    )
    .expect("initialize compute control store");
    let reconciled = compute
        .reconcile(&instances, configuration.bound_instance_id.as_deref())
        .expect("reconcile desktop inventory");
    assert_eq!(reconciled.len(), instances.len());
}

#[tokio::test]
#[ignore = "requires the configured paid GPU instance and runs Qwen Image plus H3 I2V"]
async fn generate_image_then_video_through_desktop_service_client() {
    let app_data_dir = PathBuf::from(required("ZHIHUA_TEST_APP_DATA_DIR"));
    let output_dir = PathBuf::from(required("ZHIHUA_TEST_OUTPUT_DIR"));
    let instance_id = required("ZHIHUA_TEST_COMPSHARE_INSTANCE_ID");
    std::fs::create_dir_all(&output_dir).expect("create validation output directory");

    let tunnel = SshTunnelManager::new(app_data_dir.clone()).expect("initialize SSH tunnel");
    let tunnel_status = tunnel
        .start_for(&instance_id)
        .await
        .expect("start SSH tunnel");
    let service = ServiceClient::new(app_data_dir).expect("initialize service client");
    service
        .retarget(tunnel_status.local_url.expect("local tunnel URL"))
        .expect("retarget service to validation tunnel");
    let probe = service.probe().await.expect("probe generation service");
    assert!(probe.comfyui_ready);

    let request_suffix = uuid::Uuid::new_v4();
    let image_job = service
        .submit_job(SubmitServiceJobInput {
            client_request_id: format!("live-image-{request_suffix}"),
            project_id: "live-e2e-validation".to_owned(),
            scene_id: "lightning-scene".to_owned(),
            kind: "image_generation".to_owned(),
            workflow_id: "qwen-image-generate-v1".to_owned(),
            parameters: json!({
                "prompt": "儿童科普插画，深蓝色雷云覆盖远山，一道明亮闪电连接云层与地面，画面清晰，主体居中，电影感光线，无文字，无水印",
                "negativePrompt": "模糊，畸形，文字，水印，低清晰度",
                "width": 1344,
                "height": 768,
                "seed": 20260912
            }),
        })
        .await
        .expect("submit image generation job");
    let image_job = wait_for_job(&service, &image_job.id, 10).await;
    let image_artifact = image_job
        .result_manifest
        .expect("image result manifest")
        .artifacts
        .into_iter()
        .find(|artifact| artifact.kind == "image")
        .expect("image artifact");
    let image_path = output_dir.join("qwen-lightning-1344x768.png");
    service
        .download_artifact(DownloadServiceArtifactInput {
            job_id: image_job.id,
            artifact_id: image_artifact.artifact_id,
            destination_path: image_path.to_string_lossy().into_owned(),
            expected_size_bytes: image_artifact.size_bytes,
            expected_sha256: image_artifact.sha256,
        })
        .await
        .expect("download generated image");

    let uploaded = service
        .upload_input(&image_path.to_string_lossy())
        .await
        .expect("upload generated image as first frame");
    let video_job = service
        .submit_job(SubmitServiceJobInput {
            client_request_id: format!("live-video-{request_suffix}"),
            project_id: "live-e2e-validation".to_owned(),
            scene_id: "lightning-scene".to_owned(),
            kind: "video_candidate".to_owned(),
            workflow_id: "h3-i2v-turbo-v1".to_owned(),
            parameters: json!({
                "prompt": "镜头缓慢向雷云推进，云层自然翻涌，闪电由云层向地面迅速延伸并短暂照亮群山，科普动画风格，画面稳定，无字幕",
                "seed": 20260912,
                "firstFrameFile": uploaded.remote_file,
                "width": 1344,
                "height": 768,
                "visibleWidth": 1344,
                "visibleHeight": 756,
                "cropX": 0,
                "cropY": 6,
                "length": 124,
                "discardH3Audio": true
            }),
        })
        .await
        .expect("submit H3 I2V job");
    let video_job = wait_for_job(&service, &video_job.id, 12).await;
    let video_artifact = video_job
        .result_manifest
        .expect("video result manifest")
        .artifacts
        .into_iter()
        .find(|artifact| artifact.kind == "video")
        .expect("video artifact");
    let video_path = output_dir.join("h3-lightning-i2v-1344x756.mp4");
    service
        .download_artifact(DownloadServiceArtifactInput {
            job_id: video_job.id,
            artifact_id: video_artifact.artifact_id,
            destination_path: video_path.to_string_lossy().into_owned(),
            expected_size_bytes: video_artifact.size_bytes,
            expected_sha256: video_artifact.sha256,
        })
        .await
        .expect("download generated video");
    service
        .delete_input(&uploaded.input_id)
        .await
        .expect("delete uploaded first frame");
    tunnel
        .stop_for(&instance_id)
        .await
        .expect("stop validation tunnel");
}

#[tokio::test]
#[ignore = "requires the configured paid GPU instance and compares H3 native ambience with H3 Chinese voiceover"]
async fn generate_h3_native_audio_comparison() {
    let app_data_dir = PathBuf::from(required("ZHIHUA_TEST_APP_DATA_DIR"));
    let output_dir = PathBuf::from(required("ZHIHUA_TEST_OUTPUT_DIR"));
    let instance_id = required("ZHIHUA_TEST_COMPSHARE_INSTANCE_ID");
    let image_path = output_dir.join("qwen-lightning-1344x768.png");
    assert!(
        image_path.is_file(),
        "run the image generation validation first"
    );
    std::fs::create_dir_all(&output_dir).expect("create validation output directory");

    let tunnel = SshTunnelManager::new(app_data_dir.clone()).expect("initialize SSH tunnel");
    let tunnel_status = tunnel
        .start_for(&instance_id)
        .await
        .expect("start SSH tunnel");
    let service = ServiceClient::new(app_data_dir).expect("initialize service client");
    service
        .retarget(tunnel_status.local_url.expect("local tunnel URL"))
        .expect("retarget service to validation tunnel");
    let probe = service.probe().await.expect("probe generation service");
    assert!(probe.comfyui_ready);

    let suffix = uuid::Uuid::new_v4();
    let cases = [
        (
            "ambience",
            20260913_u64,
            "For the target video, at 0.00 seconds into the target video, <Picture 1> (from [Shot 1]) is fully referenced.\n\nintegrated_multimodal_description: [Shot 1] 2D educational animation, preserve the storm clouds, mountains, colors, and composition from <Picture 1>. The camera slowly pushes toward the clouds. At 00:02.300, one bright lightning bolt strikes from the cloud to the ground and briefly lights the mountains. There are no people, no human voices, no dialogue, and no singing.\n\noverall_soundscape: Steady rain and low wind continue across the landscape. A sharp thunder crack occurs exactly with the lightning strike at 00:02.300, followed by a short natural rumble. No speech or human vocal sounds.\n\nnon_diegetic_music: N/A.",
        ),
        (
            "chinese-voiceover",
            20260914_u64,
            "For the target video, at 0.00 seconds into the target video, <Picture 1> (from [Shot 1]) is fully referenced.\n\nintegrated_multimodal_description: [Shot 1] 2D educational animation, preserve the storm clouds, mountains, colors, and composition from <Picture 1>. The camera slowly pushes toward the clouds while one lightning bolt illuminates the landscape. A calm adult Chinese narrator with a clear standard Mandarin voice (S1) says in an off-screen voiceover: <d>[Chinese] 电势差足够大时，空气会被击穿，形成闪电。</d>\n\noverall_soundscape: Soft rain and distant thunder remain low beneath the off-screen narration.\n\nnon_diegetic_music: N/A.",
        ),
    ];
    for (label, seed, prompt) in cases {
        let destination = output_dir.join(format!("h3-lightning-{label}-1344x756.mp4"));
        if destination.is_file() {
            continue;
        }
        let uploaded = service
            .upload_input(&image_path.to_string_lossy())
            .await
            .unwrap_or_else(|error| panic!("upload first frame for H3 {label}: {error:?}"));
        let job = service
            .submit_job(SubmitServiceJobInput {
                client_request_id: format!("live-h3-audio-{label}-{suffix}"),
                project_id: "live-audio-validation".to_owned(),
                scene_id: format!("lightning-{label}"),
                kind: "video_candidate".to_owned(),
                workflow_id: "h3-i2v-turbo-v1".to_owned(),
                parameters: json!({
                    "prompt": prompt,
                    "seed": seed,
                    "firstFrameFile": uploaded.remote_file,
                    "width": 1344,
                    "height": 768,
                    "visibleWidth": 1344,
                    "visibleHeight": 756,
                    "cropX": 0,
                    "cropY": 6,
                    "length": 124,
                    "discardH3Audio": false
                }),
            })
            .await
            .unwrap_or_else(|error| panic!("submit H3 {label} job: {error:?}"));
        let job = wait_for_job(&service, &job.id, 12).await;
        let artifact = job
            .result_manifest
            .unwrap_or_else(|| panic!("{label} result manifest"))
            .artifacts
            .into_iter()
            .find(|artifact| artifact.kind == "video")
            .unwrap_or_else(|| panic!("{label} video artifact"));
        service
            .download_artifact(DownloadServiceArtifactInput {
                job_id: job.id,
                artifact_id: artifact.artifact_id,
                destination_path: destination.to_string_lossy().into_owned(),
                expected_size_bytes: artifact.size_bytes,
                expected_sha256: artifact.sha256,
            })
            .await
            .unwrap_or_else(|error| panic!("download H3 {label}: {error:?}"));
        let _ = service.delete_input(&uploaded.input_id).await;
    }
    tunnel
        .stop_for(&instance_id)
        .await
        .expect("stop validation tunnel");
}
