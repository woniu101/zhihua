use crate::asset::AssetStorage;
use crate::comp_share::{
    BindCompShareInstanceInput, CompShareInstanceLocator, CompSharePowerState, CompShareProvider,
    CompShareRunningMode, CompShareStartMode, ListCompShareInstancesInput,
    SaveCompShareCredentialsInput,
};
use crate::export::{
    EnvironmentAudioPolicy, ExportProjectInput, FfmpegExporter, OutputRendition, SubtitleMode,
};
use crate::frame_profile::FrameAspectRatio;
use crate::generation::{GenerationStorage, RecordCandidateInput, RecordEnhancedInput};
use crate::service::{
    DownloadServiceArtifactInput, SaveServiceConnectionInput, ServiceClient, ServiceJob,
    SubmitServiceJobInput,
};
use crate::ssh_tunnel::{SaveTunnelConfigurationInput, SshTunnelManager};
use crate::storage::{CreateProjectInput, ProjectStorage};
use crate::storyboard::{
    AudioIntent, CandidateQuality, GenerationMode, NarrationMode, PromptMode, SceneDraft,
    SceneStatus, StoryboardStorage, VisualIntent,
};
use crate::tts::{SynthesizeNarrationInput, SystemTtsProvider};
use serde_json::json;
use std::path::PathBuf;

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name}"))
}

async fn wait_for_job(
    service: &ServiceClient,
    instance_id: &str,
    id: &str,
    max_minutes: u64,
) -> ServiceJob {
    for _ in 0..(max_minutes * 20) {
        let job = service
            .get_job_for(instance_id, id)
            .await
            .expect("poll remote job");
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

async fn start_saved_tunnel_with_retry(
    tunnel: &SshTunnelManager,
    instance_id: &str,
) -> crate::ssh_tunnel::TunnelStatus {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(150);
    loop {
        match tunnel.start_for(instance_id).await {
            Ok(status) => return status,
            Err(error) if tokio::time::Instant::now() < deadline => {
                eprintln!("waiting for SSH after instance start: {}", error.message);
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            Err(error) => panic!(
                "SSH did not recover after instance start: {}",
                error.message
            ),
        }
    }
}

#[tokio::test]
#[ignore = "stops one saved CompShare instance after live validation and verifies the stopped state"]
async fn stop_saved_instance_after_live_validation() {
    let app_data_dir = PathBuf::from(required("ZHIHUA_TEST_APP_DATA_DIR"));
    let instance_id = required("ZHIHUA_TEST_COMPSHARE_INSTANCE_ID");
    let provider = CompShareProvider::new(app_data_dir).expect("initialize CompShare");
    let platform = provider
        .list_instances(ListCompShareInstancesInput {
            region: None,
            zone: None,
        })
        .await
        .expect("list instances")
        .into_iter()
        .find(|instance| instance.instance_id == instance_id)
        .expect("saved instance exists");
    let locator = CompShareInstanceLocator {
        instance_id,
        region: platform.region,
        zone: platform.zone,
        project_id: platform.project_id,
    };
    let current = provider
        .describe_instance(locator.clone())
        .await
        .expect("read instance state");
    if current.state != CompSharePowerState::Stopped {
        provider
            .stop_instance_for(locator.clone())
            .await
            .expect("request instance stop");
    }
    let stopped = crate::wait_for_instance_state_for(
        &provider,
        &locator,
        CompSharePowerState::Stopped,
        None,
        std::time::Duration::from_secs(180),
    )
    .await
    .expect("wait for stopped instance");
    assert_eq!(stopped.state, CompSharePowerState::Stopped);
}

#[tokio::test]
#[ignore = "uses a running saved worker to validate production artifact Range resume"]
async fn resume_real_artifact_from_saved_partial() {
    let app_data_dir = PathBuf::from(required("ZHIHUA_TEST_APP_DATA_DIR"));
    let output_dir = PathBuf::from(required("ZHIHUA_TEST_OUTPUT_DIR"));
    let instance_id = required("ZHIHUA_TEST_COMPSHARE_INSTANCE_ID");
    std::fs::create_dir_all(&output_dir).expect("create recovery output directory");
    let destination = output_dir.join("range-resumed-remote-artifact.bin");
    let partial = destination.with_file_name(format!(
        ".{}.zhihua-part",
        destination.file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_file(&destination);
    let _ = std::fs::remove_file(&partial);

    let tunnel = SshTunnelManager::new(app_data_dir.clone()).expect("initialize SSH tunnel");
    let status = start_saved_tunnel_with_retry(&tunnel, &instance_id).await;
    tunnel
        .ensure_remote_service_for(&instance_id)
        .await
        .expect("ensure remote service");
    let service = ServiceClient::new(app_data_dir).expect("initialize service client");
    service
        .retarget_for(&instance_id, status.local_url.expect("recovery tunnel URL"))
        .expect("retarget recovery service");
    let job = service
        .list_jobs_for(&instance_id, 100)
        .await
        .expect("list remote jobs")
        .into_iter()
        .filter(|job| job.status == "completed")
        .max_by_key(|job| {
            job.result_manifest
                .as_ref()
                .and_then(|manifest| manifest.artifacts.iter().map(|item| item.size_bytes).max())
                .unwrap_or(0)
        })
        .expect("running worker has a completed job");
    let artifact = job
        .result_manifest
        .as_ref()
        .and_then(|manifest| manifest.artifacts.iter().max_by_key(|item| item.size_bytes))
        .expect("completed job has an artifact");
    service
        .download_artifact_for(
            &instance_id,
            DownloadServiceArtifactInput {
                job_id: job.id.clone(),
                artifact_id: artifact.artifact_id.clone(),
                destination_path: destination.to_string_lossy().into_owned(),
                expected_size_bytes: artifact.size_bytes,
                expected_sha256: artifact.sha256.clone(),
            },
        )
        .await
        .expect("download baseline artifact");
    let baseline = std::fs::read(&destination).expect("read baseline artifact");
    assert!(baseline.len() > 1_048_576, "artifact must exceed one MiB");
    std::fs::remove_file(&destination).expect("remove baseline destination");
    std::fs::write(&partial, &baseline[..1_048_576]).expect("seed interrupted partial");
    let resumed = service
        .download_artifact_for(
            &instance_id,
            DownloadServiceArtifactInput {
                job_id: job.id,
                artifact_id: artifact.artifact_id.clone(),
                destination_path: destination.to_string_lossy().into_owned(),
                expected_size_bytes: artifact.size_bytes,
                expected_sha256: artifact.sha256.clone(),
            },
        )
        .await
        .expect("resume real remote artifact");
    assert!(resumed.resumed, "server must honor Range resume");
    assert_eq!(
        std::fs::read(&destination).expect("read resumed artifact"),
        baseline
    );
    tunnel
        .stop_for(&instance_id)
        .await
        .expect("stop validation tunnel");
}

#[tokio::test]
#[ignore = "uses saved desktop credentials to validate real Range resume and persistence across a no-GPU instance restart"]
async fn recover_real_artifact_after_client_exit_and_instance_restart() {
    let app_data_dir = PathBuf::from(required("ZHIHUA_TEST_APP_DATA_DIR"));
    let output_dir = PathBuf::from(required("ZHIHUA_TEST_OUTPUT_DIR"));
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
        .expect("recovery instance exists");
    let locator = CompShareInstanceLocator {
        instance_id: instance_id.clone(),
        region: platform.region,
        zone: platform.zone,
        project_id: platform.project_id,
    };
    assert!(
        platform.support_without_gpu_start,
        "instance must support no-GPU restart"
    );

    let current = provider
        .describe_instance(locator.clone())
        .await
        .expect("read initial instance state");
    if matches!(
        current.state,
        CompSharePowerState::Running | CompSharePowerState::Starting
    ) {
        provider
            .stop_instance_for(locator.clone())
            .await
            .expect("stop instance before recovery validation");
        crate::wait_for_instance_state_for(
            &provider,
            &locator,
            CompSharePowerState::Stopped,
            None,
            std::time::Duration::from_secs(180),
        )
        .await
        .expect("wait for stopped instance");
    }

    std::fs::create_dir_all(&output_dir).expect("create recovery output directory");
    let destination = output_dir.join("recovered-remote-artifact.mp4");
    let partial = destination.with_file_name(format!(
        ".{}.zhihua-part",
        destination.file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_file(&destination);
    let _ = std::fs::remove_file(&partial);

    let validation = async {
        provider
            .start_instance_for(locator.clone(), CompShareStartMode::NoGpu)
            .await
            .map_err(|error| error.message)?;
        crate::wait_for_instance_state_for(
            &provider,
            &locator,
            CompSharePowerState::Running,
            Some(CompShareRunningMode::NoGpu),
            std::time::Duration::from_secs(180),
        )
        .await?;

        let tunnel = SshTunnelManager::new(app_data_dir.clone()).map_err(|error| error.message)?;
        let first_tunnel = start_saved_tunnel_with_retry(&tunnel, &instance_id).await;
        tunnel
            .ensure_remote_service_for(&instance_id)
            .await
            .map_err(|error| error.message)?;
        let service = ServiceClient::new(app_data_dir.clone())?;
        service
            .retarget_for(
                &instance_id,
                first_tunnel.local_url.expect("first recovery tunnel URL"),
            )
            .map_err(|error| error.message)?;
        let first_job = service
            .list_jobs_for(&instance_id, 100)
            .await
            .map_err(|error| error.message)?
            .into_iter()
            .filter(|job| job.status == "completed")
            .max_by_key(|job| {
                job.result_manifest
                    .as_ref()
                    .and_then(|manifest| {
                        manifest.artifacts.iter().map(|item| item.size_bytes).max()
                    })
                    .unwrap_or(0)
            })
            .ok_or_else(|| {
                "instance has no completed job available for recovery validation".to_owned()
            })?;
        let artifact = first_job
            .result_manifest
            .clone()
            .and_then(|manifest| {
                manifest
                    .artifacts
                    .into_iter()
                    .max_by_key(|item| item.size_bytes)
            })
            .ok_or_else(|| "recovery job has no downloadable artifact".to_owned())?;

        let baseline = service
            .download_artifact_for(
                &instance_id,
                DownloadServiceArtifactInput {
                    job_id: first_job.id.clone(),
                    artifact_id: artifact.artifact_id.clone(),
                    destination_path: destination.to_string_lossy().into_owned(),
                    expected_size_bytes: artifact.size_bytes,
                    expected_sha256: artifact.sha256.clone(),
                },
            )
            .await
            .map_err(|error| error.message)?;
        if baseline.size_bytes != artifact.size_bytes {
            return Err("baseline remote download size differs from manifest".to_owned());
        }
        let source_bytes = std::fs::read(&destination).map_err(|error| error.to_string())?;
        if source_bytes.len() <= 1_048_576 {
            return Err("recovery artifact is too small for a meaningful resume".to_owned());
        }
        std::fs::remove_file(&destination).map_err(|error| error.to_string())?;

        // A forced desktop exit leaves this hidden partial file behind. Seed the exact
        // first MiB and verify the production server honors Range on the next process.
        std::fs::write(&partial, &source_bytes[..1_048_576]).map_err(|error| error.to_string())?;
        let resumed = service
            .download_artifact_for(
                &instance_id,
                DownloadServiceArtifactInput {
                    job_id: first_job.id.clone(),
                    artifact_id: artifact.artifact_id.clone(),
                    destination_path: destination.to_string_lossy().into_owned(),
                    expected_size_bytes: artifact.size_bytes,
                    expected_sha256: artifact.sha256.clone(),
                },
            )
            .await
            .map_err(|error| error.message)?;
        if !resumed.resumed
            || std::fs::read(&destination).map_err(|error| error.to_string())? != source_bytes
        {
            return Err(
                "real remote download did not resume to the known-good artifact".to_owned(),
            );
        }

        tunnel
            .stop_for(&instance_id)
            .await
            .map_err(|error| error.message)?;
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
        .await?;
        provider
            .start_instance_for(locator.clone(), CompShareStartMode::NoGpu)
            .await
            .map_err(|error| error.message)?;
        crate::wait_for_instance_state_for(
            &provider,
            &locator,
            CompSharePowerState::Running,
            Some(CompShareRunningMode::NoGpu),
            std::time::Duration::from_secs(180),
        )
        .await?;

        let second_tunnel = start_saved_tunnel_with_retry(&tunnel, &instance_id).await;
        tunnel
            .ensure_remote_service_for(&instance_id)
            .await
            .map_err(|error| error.message)?;
        service
            .retarget_for(
                &instance_id,
                second_tunnel.local_url.expect("second recovery tunnel URL"),
            )
            .map_err(|error| error.message)?;
        let recovered_job = service
            .get_job_for(&instance_id, &first_job.id)
            .await
            .map_err(|error| error.message)?;
        if recovered_job.status != "completed" || recovered_job.result_manifest.is_none() {
            return Err("completed remote job did not survive the instance restart".to_owned());
        }
        Ok::<(), String>(())
    }
    .await;

    let cleanup_tunnel = SshTunnelManager::new(app_data_dir).expect("initialize cleanup tunnel");
    let _ = cleanup_tunnel.stop_for(&instance_id).await;
    if let Ok(current) = provider.describe_instance(locator.clone()).await {
        if matches!(
            current.state,
            CompSharePowerState::Running | CompSharePowerState::Starting
        ) {
            let _ = provider.stop_instance_for(locator.clone()).await;
            let _ = crate::wait_for_instance_state_for(
                &provider,
                &locator,
                CompSharePowerState::Stopped,
                None,
                std::time::Duration::from_secs(180),
            )
            .await;
        }
    }
    validation.expect("real download and instance restart recovery");
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

        let bootstrap_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let access = provider
                .ssh_access_for(locator.clone())
                .await
                .map_err(|error| error.message)?;
            match tunnel
                .bootstrap_configuration(
                    &instance_id,
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
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
                Err(error) => return Err(error.message),
            }
        }
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
        .map_err(|error| error.to_string())?;
        provider
            .delete_stop_scheduler_for(locator.clone())
            .await
            .map_err(|error| error.message)?;
        Ok::<_, String>(())
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
        .retarget_for(
            &instance_id,
            tunnel_status.local_url.expect("local tunnel URL"),
        )
        .expect("retarget service to validation tunnel");
    let probe = service
        .probe_for(&instance_id)
        .await
        .expect("probe generation service");
    assert!(probe.comfyui_ready);

    let request_suffix = uuid::Uuid::new_v4();
    let image_job = service
        .submit_job_for(
            &instance_id,
            SubmitServiceJobInput {
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
            },
        )
        .await
        .expect("submit image generation job");
    let image_job = wait_for_job(&service, &instance_id, &image_job.id, 10).await;
    let image_artifact = image_job
        .result_manifest
        .expect("image result manifest")
        .artifacts
        .into_iter()
        .find(|artifact| artifact.kind == "image")
        .expect("image artifact");
    let image_path = output_dir.join("qwen-lightning-1344x768.png");
    service
        .download_artifact_for(
            &instance_id,
            DownloadServiceArtifactInput {
                job_id: image_job.id,
                artifact_id: image_artifact.artifact_id,
                destination_path: image_path.to_string_lossy().into_owned(),
                expected_size_bytes: image_artifact.size_bytes,
                expected_sha256: image_artifact.sha256,
            },
        )
        .await
        .expect("download generated image");

    let uploaded = service
        .upload_input_for(&instance_id, &image_path.to_string_lossy())
        .await
        .expect("upload generated image as first frame");
    let video_job = service
        .submit_job_for(
            &instance_id,
            SubmitServiceJobInput {
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
            },
        )
        .await
        .expect("submit H3 I2V job");
    let video_job = wait_for_job(&service, &instance_id, &video_job.id, 12).await;
    let video_artifact = video_job
        .result_manifest
        .expect("video result manifest")
        .artifacts
        .into_iter()
        .find(|artifact| artifact.kind == "video")
        .expect("video artifact");
    let video_path = output_dir.join("h3-lightning-i2v-1344x756.mp4");
    service
        .download_artifact_for(
            &instance_id,
            DownloadServiceArtifactInput {
                job_id: video_job.id,
                artifact_id: video_artifact.artifact_id,
                destination_path: video_path.to_string_lossy().into_owned(),
                expected_size_bytes: video_artifact.size_bytes,
                expected_sha256: video_artifact.sha256,
            },
        )
        .await
        .expect("download generated video");
    service
        .delete_input_for(&instance_id, &uploaded.input_id)
        .await
        .expect("delete uploaded first frame");
    tunnel
        .stop_for(&instance_id)
        .await
        .expect("stop validation tunnel");
}

#[tokio::test]
#[ignore = "requires the configured paid GPU instance and exports a real 30-second project"]
async fn generate_and_export_real_multiscene_project() {
    let app_data_dir = PathBuf::from(required("ZHIHUA_TEST_APP_DATA_DIR"));
    let artifact_root = PathBuf::from(required("ZHIHUA_TEST_OUTPUT_DIR"))
        .join(format!("full-flow-{}", uuid::Uuid::new_v4()));
    let instance_id = required("ZHIHUA_TEST_COMPSHARE_INSTANCE_ID");
    let database_path = artifact_root.join("acceptance.sqlite3");
    let projects_root = artifact_root.join("projects");
    let exports_root = artifact_root.join("exports");
    std::fs::create_dir_all(&exports_root).expect("create full-flow artifact directory");

    let projects = ProjectStorage::initialize(&database_path, &projects_root)
        .expect("initialize isolated project storage");
    let project = projects
        .create_project(CreateProjectInput {
            title: "雷电是怎样形成的（真实闭环验收）".to_owned(),
            audience: Some("8 至 14 岁科普学习者".to_owned()),
            target_duration_sec: Some(30),
        })
        .expect("create acceptance project");
    let storyboards =
        StoryboardStorage::initialize(projects.clone()).expect("initialize storyboard storage");
    let generations =
        GenerationStorage::initialize(projects.clone()).expect("initialize generation storage");
    let assets = AssetStorage::initialize(projects.clone()).expect("initialize asset storage");
    let tts = SystemTtsProvider::initialize(projects.clone()).expect("initialize system TTS");
    let exporter = FfmpegExporter::new(projects.clone());
    assert!(exporter.capability().available, "FFmpeg must be available");

    let scene_specs = [
        (
            "云层中的电荷",
            "雷雨云里，冰晶和水滴不断碰撞，使正负电荷逐渐分离。",
            "雷雨云内部的冰晶与水滴碰撞，蓝紫色云层中正负电荷粒子清晰分层，远处群山，儿童科普插画，电影感光线，画面清晰，无文字，无水印",
            "For the target video, at 0.00 seconds into the target video, <Picture 1> (from [Shot 1]) is fully referenced.\n\nintegrated_multimodal_description: [Shot 1] Preserve the educational illustration, cloud structure, mountains, colors, and composition from <Picture 1>. The camera slowly moves through the storm cloud while small ice crystals and droplets circulate naturally. Positive and negative charge particles separate into two layers. Smooth stable motion, no text, no people, no human voices, no dialogue, no singing.\n\noverall_soundscape: Soft high-altitude wind and subtle ice particle impacts continue naturally. No speech or human vocal sounds.\n\nnon_diegetic_music: N/A.",
            "高空风声与细小冰晶碰撞声",
            "电荷在云层中逐渐分离",
        ),
        (
            "空气被击穿",
            "当电势差足够大，空气会被击穿，形成一条导电通道。",
            "深蓝雷雨云与地面之间形成弯曲的发光先导通道，电荷沿通道向下延伸，远山夜景，儿童科普插画，电影感光线，无文字，无水印",
            "For the target video, at 0.00 seconds into the target video, <Picture 1> (from [Shot 1]) is fully referenced.\n\nintegrated_multimodal_description: [Shot 1] Preserve the storm cloud, landscape, palette, and composition from <Picture 1>. The camera holds a wide view as a branching luminous leader gradually extends from the cloud toward the ground, revealing a conductive path through the air. Stable educational animation, no text, no people, no human voices, no dialogue, no singing.\n\noverall_soundscape: Steady rain and low wind, with a faint rising electrical crackle synchronized with the growing channel. No speech or human vocal sounds.\n\nnon_diegetic_music: N/A.",
            "雨声、低风声与逐渐增强的电流噼啪声",
            "强电场击穿空气",
        ),
        (
            "闪电与雷声",
            "导电通道接通后，强电流瞬间通过，耀眼的闪电和雷声随之出现。",
            "一道明亮闪电从厚重雷云连接地面并照亮群山，雨幕和云层细节丰富，儿童科普插画，电影感构图，无文字，无水印",
            "For the target video, at 0.00 seconds into the target video, <Picture 1> (from [Shot 1]) is fully referenced.\n\nintegrated_multimodal_description: [Shot 1] Preserve the storm clouds, mountains, rain, colors, and composition from <Picture 1>. The camera slowly pushes forward. At 00:04.000 a brilliant lightning discharge races through the established channel to the ground and briefly illuminates the entire landscape. Stable cinematic educational animation, no text, no people, no human voices, no dialogue, no singing.\n\noverall_soundscape: Rain and wind continue. A sharp thunder crack occurs exactly with the flash at 00:04.000, followed by a natural low rumble that fades into the rain. No speech or human vocal sounds.\n\nnon_diegetic_music: N/A.",
            "持续雨声、闪电同步的雷击声与低沉回响",
            "强电流产生闪电和雷声",
        ),
    ];

    let mut scenes = Vec::new();
    for (order, (title, narration, _, _, ambient, caption)) in scene_specs.iter().enumerate() {
        scenes.push(
            storyboards
                .upsert(SceneDraft {
                    id: uuid::Uuid::new_v4().to_string(),
                    project_id: project.id.clone(),
                    order: order as u32,
                    title: (*title).to_owned(),
                    purpose: "用一个十秒镜头说明雷电形成过程".to_owned(),
                    source_refs: vec![],
                    narration: (*narration).to_owned(),
                    narration_mode: NarrationMode::Tts,
                    ambient_sound: (*ambient).to_owned(),
                    on_screen_text: vec![(*caption).to_owned()],
                    visual_plan: scene_specs[order].2.to_owned(),
                    visual_intent: VisualIntent {
                        subject: (*title).to_owned(),
                        action: "自然、连续地展示物理过程".to_owned(),
                        scene: "雷雨云与远山".to_owned(),
                        composition: "16:9 科普电影画面".to_owned(),
                        camera: "缓慢推进或稳定广角".to_owned(),
                        lighting: "深蓝环境光与闪电高光".to_owned(),
                        timeline: "十秒内完成一个清晰变化".to_owned(),
                        negative: "文字，水印，人物，人声，抖动，畸变".to_owned(),
                    },
                    prompt_mode: PromptMode::Advanced,
                    audio_intent: AudioIntent::Environment,
                    locked: false,
                    generation_mode: GenerationMode::I2v,
                    target_duration_ms: 10_000,
                    asset_ids: vec![],
                    selected_version_id: None,
                    last_job_id: None,
                    last_upscale_job_id: None,
                    pending_request_id: None,
                    generation_stage: None,
                    status: SceneStatus::Ready,
                    quality: CandidateQuality::Fast,
                    updated_at: String::new(),
                })
                .expect("persist acceptance scene"),
        );
    }

    let tunnel = SshTunnelManager::new(app_data_dir.clone()).expect("initialize SSH tunnel");
    let tunnel_status = tunnel
        .start_for(&instance_id)
        .await
        .expect("start full-flow SSH tunnel");
    let service = ServiceClient::new(app_data_dir).expect("initialize service client");
    service
        .retarget_for(
            &instance_id,
            tunnel_status.local_url.expect("full-flow local tunnel URL"),
        )
        .expect("retarget service to full-flow tunnel");
    let probe = service
        .probe_for(&instance_id)
        .await
        .expect("probe full-flow generation service");
    assert!(probe.comfyui_ready);

    for (index, scene) in scenes.iter().enumerate() {
        let (_, _, image_prompt, video_prompt, _, _) = scene_specs[index];
        let suffix = uuid::Uuid::new_v4();
        let image_job = service
            .submit_job_for(
                &instance_id,
                SubmitServiceJobInput {
                    client_request_id: format!("full-flow-image-{suffix}"),
                    project_id: project.id.clone(),
                    scene_id: scene.id.clone(),
                    kind: "image_generation".to_owned(),
                    workflow_id: "qwen-image-generate-v1".to_owned(),
                    parameters: json!({
                        "prompt": image_prompt,
                        "negativePrompt": "模糊，畸形，文字，水印，低清晰度，人物",
                        "width": 1344,
                        "height": 768,
                        "seed": 2026091401_u64 + index as u64
                    }),
                },
            )
            .await
            .expect("submit scene image job");
        let image_job = wait_for_job(&service, &instance_id, &image_job.id, 12).await;
        let image_artifact = image_job
            .result_manifest
            .expect("scene image result manifest")
            .artifacts
            .into_iter()
            .find(|artifact| artifact.kind == "image")
            .expect("scene image artifact");
        let image_path = project
            .project_dir
            .join("assets")
            .join(format!("scene-{}-first-frame.png", index + 1));
        service
            .download_artifact_for(
                &instance_id,
                DownloadServiceArtifactInput {
                    job_id: image_job.id,
                    artifact_id: image_artifact.artifact_id,
                    destination_path: image_path.to_string_lossy().into_owned(),
                    expected_size_bytes: image_artifact.size_bytes,
                    expected_sha256: image_artifact.sha256,
                },
            )
            .await
            .expect("download scene image");

        let uploaded = service
            .upload_input_for(&instance_id, &image_path.to_string_lossy())
            .await
            .expect("upload scene first frame");
        let video_job = service
            .submit_job_for(
                &instance_id,
                SubmitServiceJobInput {
                    client_request_id: format!("full-flow-video-{suffix}"),
                    project_id: project.id.clone(),
                    scene_id: scene.id.clone(),
                    kind: "video_candidate".to_owned(),
                    workflow_id: "h3-i2v-turbo-v1".to_owned(),
                    parameters: json!({
                        "prompt": video_prompt,
                        "seed": 2026091411_u64 + index as u64,
                        "firstFrameFile": uploaded.remote_file,
                        "width": 1344,
                        "height": 768,
                        "visibleWidth": 1344,
                        "visibleHeight": 756,
                        "cropX": 0,
                        "cropY": 6,
                        "length": 243,
                        "discardH3Audio": false
                    }),
                },
            )
            .await
            .expect("submit scene H3 job");

        if index == 0 {
            tunnel
                .stop_for(&instance_id)
                .await
                .expect("simulate client tunnel loss");
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
            let resumed = tunnel
                .start_for(&instance_id)
                .await
                .expect("restore tunnel after simulated loss");
            service
                .retarget_for(
                    &instance_id,
                    resumed.local_url.expect("restored tunnel URL"),
                )
                .expect("retarget after tunnel recovery");
            assert!(
                service
                    .probe_for(&instance_id)
                    .await
                    .expect("probe after tunnel recovery")
                    .compatible
            );
        }

        let video_job = wait_for_job(&service, &instance_id, &video_job.id, 15).await;
        let prompt_id = video_job.prompt_id.clone();
        let video_artifact = video_job
            .result_manifest
            .expect("scene video result manifest")
            .artifacts
            .into_iter()
            .find(|artifact| artifact.kind == "video")
            .expect("scene video artifact");
        let video_path = project
            .project_dir
            .join("cache")
            .join("drafts")
            .join(format!("scene-{}-candidate.mp4", index + 1));
        let downloaded = service
            .download_artifact_for(
                &instance_id,
                DownloadServiceArtifactInput {
                    job_id: video_job.id.clone(),
                    artifact_id: video_artifact.artifact_id.clone(),
                    destination_path: video_path.to_string_lossy().into_owned(),
                    expected_size_bytes: video_artifact.size_bytes,
                    expected_sha256: video_artifact.sha256.clone(),
                },
            )
            .await
            .expect("download scene video");
        service
            .delete_input_for(&instance_id, &uploaded.input_id)
            .await
            .expect("delete uploaded scene first frame");
        let candidate = generations
            .record(RecordCandidateInput {
                project_id: project.id.clone(),
                scene_id: scene.id.clone(),
                job_id: video_job.id,
                workflow_id: "h3-i2v-turbo-v1".to_owned(),
                prompt_id,
                prompt_compiler_version: Some("h3-prompt-v2".to_owned()),
                h3_audio_policy: Some("smart".to_owned()),
                prompt_text: Some(video_prompt.to_owned()),
                seed: Some(2026091411_u32 + index as u32),
                audio_intent: Some("environment".to_owned()),
                target_duration_sec: Some(10),
                artifact_id: video_artifact.artifact_id,
                filename: video_artifact.filename,
                media_type: video_artifact.media_type,
                local_path: video_path,
                size_bytes: downloaded.size_bytes,
                sha256: downloaded.sha256,
                aspect_ratio: "16:9".to_owned(),
                work_width: 1344,
                work_height: 768,
                visible_width: 1344,
                visible_height: 756,
                crop_x: 0,
                crop_y: 6,
            })
            .expect("record generated candidate");
        generations
            .select(&project.id, &scene.id, &candidate.id)
            .expect("select official candidate");
    }

    tunnel
        .stop_for(&instance_id)
        .await
        .expect("stop full-flow validation tunnel");

    let voice = tts
        .list_voices()
        .expect("list system voices")
        .into_iter()
        .find(|voice| voice.locale.starts_with("zh"))
        .expect("Chinese system voice is required for acceptance");
    for scene in &scenes {
        let narration = tts
            .synthesize(
                &storyboards,
                SynthesizeNarrationInput {
                    project_id: project.id.clone(),
                    scene_id: scene.id.clone(),
                    voice_id: voice.id.clone(),
                    rate: 0,
                    volume: 100,
                },
            )
            .expect("synthesize scene narration");
        assert!(
            narration.duration_ms < 10_000,
            "narration must fit its scene"
        );
    }

    let export = exporter
        .export(
            &storyboards,
            &generations,
            &assets,
            &tts,
            ExportProjectInput {
                project_id: project.id.clone(),
                output_directory: exports_root,
                frame_rate: 24,
                subtitle_mode: SubtitleMode::BurnAndSrt,
                aspect_ratio: FrameAspectRatio::Landscape,
                rendition: OutputRendition::Candidate,
                narration_volume: 82,
                music_asset_id: None,
                music_volume: 0,
                music_fade: false,
                environment_audio_policy: EnvironmentAudioPolicy::Smart,
                environment_volume: 32,
            },
        )
        .expect("export complete acceptance video");
    assert_eq!((export.width, export.height), (1344, 756));
    assert!((29_000..=31_000).contains(&export.duration_ms));
    assert!(export.output_path.is_file());
    assert!(export
        .subtitle_path
        .as_ref()
        .is_some_and(|path| path.is_file()));
    std::fs::write(
        artifact_root.join("acceptance-result.json"),
        serde_json::to_vec_pretty(&json!({
            "projectId": project.id,
            "projectDirectory": project.project_dir,
            "outputPath": export.output_path,
            "subtitlePath": export.subtitle_path,
            "durationMs": export.duration_ms,
            "width": export.width,
            "height": export.height,
            "sizeBytes": export.size_bytes,
            "sha256": export.sha256,
            "faultInjection": "first H3 job survived an SSH tunnel stop and reconnect"
        }))
        .expect("serialize acceptance result"),
    )
    .expect("write acceptance result");
}

#[tokio::test]
#[ignore = "requires the configured paid GPU instance and enhances a completed acceptance project to 1080p"]
async fn enhance_and_export_real_multiscene_project_to_1080p() {
    let app_data_dir = PathBuf::from(required("ZHIHUA_TEST_APP_DATA_DIR"));
    let run_dir = PathBuf::from(required("ZHIHUA_TEST_FULL_FLOW_RUN_DIR"));
    let instance_id = required("ZHIHUA_TEST_COMPSHARE_INSTANCE_ID");
    let acceptance: serde_json::Value = serde_json::from_slice(
        &std::fs::read(run_dir.join("acceptance-result.json"))
            .expect("read candidate acceptance result"),
    )
    .expect("parse candidate acceptance result");
    let project_id = acceptance["projectId"]
        .as_str()
        .expect("acceptance project ID")
        .to_owned();

    let projects =
        ProjectStorage::initialize(run_dir.join("acceptance.sqlite3"), run_dir.join("projects"))
            .expect("open acceptance project storage");
    let project = projects
        .get_project(&project_id)
        .expect("open acceptance project");
    let storyboards = StoryboardStorage::initialize(projects.clone())
        .expect("open acceptance storyboard storage");
    let generations = GenerationStorage::initialize(projects.clone())
        .expect("open acceptance generation storage");
    let assets = AssetStorage::initialize(projects.clone()).expect("open acceptance asset storage");
    let tts = SystemTtsProvider::initialize(projects.clone()).expect("open acceptance TTS");
    let scenes = storyboards
        .list(&project_id)
        .expect("list acceptance scenes");
    assert_eq!(scenes.len(), 3, "acceptance project must have three scenes");

    let tunnel = SshTunnelManager::new(app_data_dir.clone()).expect("initialize SSH tunnel");
    let tunnel_status = tunnel
        .start_for(&instance_id)
        .await
        .expect("start enhancement SSH tunnel");
    let service = ServiceClient::new(app_data_dir).expect("initialize service client");
    service
        .retarget_for(
            &instance_id,
            tunnel_status
                .local_url
                .expect("enhancement local tunnel URL"),
        )
        .expect("retarget enhancement service");
    let probe = service
        .probe_for(&instance_id)
        .await
        .expect("probe enhancement service");
    assert!(
        probe
            .available_workflows
            .iter()
            .any(|workflow| workflow == "seedvr2-1080p-v1"),
        "SeedVR2 1080p workflow must be available"
    );

    for (index, scene) in scenes.iter().enumerate() {
        let candidate = generations
            .list(&project_id, &scene.id)
            .expect("list scene candidates")
            .into_iter()
            .find(|candidate| candidate.selected)
            .expect("selected scene candidate");
        let uploaded = service
            .upload_input_for(&instance_id, &candidate.local_path.to_string_lossy())
            .await
            .expect("upload selected candidate for enhancement");
        let suffix = uuid::Uuid::new_v4();
        let job = service
            .submit_job_for(
                &instance_id,
                SubmitServiceJobInput {
                    client_request_id: format!("full-flow-upscale-{suffix}"),
                    project_id: project_id.clone(),
                    scene_id: scene.id.clone(),
                    kind: "video_upscale".to_owned(),
                    workflow_id: "seedvr2-1080p-v1".to_owned(),
                    parameters: json!({
                        "sourceVideoFile": uploaded.remote_file,
                        "seed": 2026091451_u64 + index as u64
                    }),
                },
            )
            .await
            .expect("submit SeedVR2 enhancement");
        let job = wait_for_job(&service, &instance_id, &job.id, 20).await;
        let prompt_id = job.prompt_id.clone();
        let artifact = job
            .result_manifest
            .expect("enhancement result manifest")
            .artifacts
            .into_iter()
            .find(|artifact| artifact.kind == "video")
            .expect("enhanced video artifact");
        let destination = project
            .project_dir
            .join("generated")
            .join("videos")
            .join(&scene.id)
            .join("1080p")
            .join(format!("scene-{}-1080p.mp4", index + 1));
        let downloaded = service
            .download_artifact_for(
                &instance_id,
                DownloadServiceArtifactInput {
                    job_id: job.id.clone(),
                    artifact_id: artifact.artifact_id.clone(),
                    destination_path: destination.to_string_lossy().into_owned(),
                    expected_size_bytes: artifact.size_bytes,
                    expected_sha256: artifact.sha256,
                },
            )
            .await
            .expect("download enhanced video");
        generations
            .record_enhanced(RecordEnhancedInput {
                project_id: project_id.clone(),
                scene_id: scene.id.clone(),
                source_candidate_id: candidate.id,
                job_id: job.id,
                workflow_id: "seedvr2-1080p-v1".to_owned(),
                prompt_id,
                artifact_id: artifact.artifact_id,
                filename: artifact.filename,
                media_type: artifact.media_type,
                local_path: downloaded.destination_path.into(),
                size_bytes: downloaded.size_bytes,
                sha256: downloaded.sha256,
            })
            .expect("record enhanced scene version");
        service
            .delete_input_for(&instance_id, &uploaded.input_id)
            .await
            .expect("delete uploaded enhancement input");
    }

    tunnel
        .stop_for(&instance_id)
        .await
        .expect("stop enhancement tunnel");
    let export = FfmpegExporter::new(projects)
        .export(
            &storyboards,
            &generations,
            &assets,
            &tts,
            ExportProjectInput {
                project_id,
                output_directory: run_dir.join("exports-1080p"),
                frame_rate: 24,
                subtitle_mode: SubtitleMode::BurnAndSrt,
                aspect_ratio: FrameAspectRatio::Landscape,
                rendition: OutputRendition::Enhanced1080p,
                narration_volume: 82,
                music_asset_id: None,
                music_volume: 0,
                music_fade: false,
                environment_audio_policy: EnvironmentAudioPolicy::Smart,
                environment_volume: 32,
            },
        )
        .expect("export enhanced acceptance video");
    assert_eq!((export.width, export.height), (1920, 1080));
    assert_eq!(export.enhanced_scene_count, 3);
    assert_eq!(export.scaled_scene_count, 0);
    assert!((29_000..=31_000).contains(&export.duration_ms));
    std::fs::write(
        run_dir.join("enhancement-result.json"),
        serde_json::to_vec_pretty(&json!({
            "outputPath": export.output_path,
            "subtitlePath": export.subtitle_path,
            "durationMs": export.duration_ms,
            "width": export.width,
            "height": export.height,
            "enhancedSceneCount": export.enhanced_scene_count,
            "scaledSceneCount": export.scaled_scene_count,
            "sizeBytes": export.size_bytes,
            "sha256": export.sha256
        }))
        .expect("serialize enhancement result"),
    )
    .expect("write enhancement result");
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
        .retarget_for(
            &instance_id,
            tunnel_status.local_url.expect("local tunnel URL"),
        )
        .expect("retarget service to validation tunnel");
    let probe = service
        .probe_for(&instance_id)
        .await
        .expect("probe generation service");
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
            .upload_input_for(&instance_id, &image_path.to_string_lossy())
            .await
            .unwrap_or_else(|error| panic!("upload first frame for H3 {label}: {error:?}"));
        let job = service
            .submit_job_for(
                &instance_id,
                SubmitServiceJobInput {
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
                },
            )
            .await
            .unwrap_or_else(|error| panic!("submit H3 {label} job: {error:?}"));
        let job = wait_for_job(&service, &instance_id, &job.id, 12).await;
        let artifact = job
            .result_manifest
            .unwrap_or_else(|| panic!("{label} result manifest"))
            .artifacts
            .into_iter()
            .find(|artifact| artifact.kind == "video")
            .unwrap_or_else(|| panic!("{label} video artifact"));
        service
            .download_artifact_for(
                &instance_id,
                DownloadServiceArtifactInput {
                    job_id: job.id,
                    artifact_id: artifact.artifact_id,
                    destination_path: destination.to_string_lossy().into_owned(),
                    expected_size_bytes: artifact.size_bytes,
                    expected_sha256: artifact.sha256,
                },
            )
            .await
            .unwrap_or_else(|error| panic!("download H3 {label}: {error:?}"));
        let _ = service
            .delete_input_for(&instance_id, &uploaded.input_id)
            .await;
    }
    tunnel
        .stop_for(&instance_id)
        .await
        .expect("stop validation tunnel");
}
