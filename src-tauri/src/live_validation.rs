use crate::comp_share::{
    BindCompShareInstanceInput, CompSharePowerState, CompShareProvider, CompShareRunningMode,
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
            host: required("ZHIHUA_TEST_SSH_HOST"),
            port: required("ZHIHUA_TEST_SSH_PORT")
                .parse::<u16>()
                .expect("valid SSH port"),
            username: "root".to_owned(),
            host_key_fingerprint: required("ZHIHUA_TEST_SSH_HOST_FINGERPRINT"),
            private_key,
        })
        .expect("save SSH tunnel configuration");
    let tunnel_status = tunnel.start().await.expect("start SSH tunnel");
    let local_url = tunnel_status.local_url.expect("local tunnel URL");
    let token = tunnel
        .read_service_token()
        .await
        .expect("read remote service token");

    let service = ServiceClient::new(app_data_dir).expect("initialize service client");
    service.clear().expect("clear stale service connection");
    service
        .save(SaveServiceConnectionInput {
            instance_id,
            base_url: local_url,
            token,
        })
        .expect("save service connection");
    let probe = service.probe().await.expect("probe service through tunnel");
    assert!(probe.compatible);
    assert!(probe.comfyui_ready);
    assert!(probe.available_workflows.len() >= 11);
    tunnel.stop().await.expect("stop validation tunnel");
}

#[tokio::test]
#[ignore = "requires the configured paid GPU instance and runs Qwen Image plus H3 I2V"]
async fn generate_image_then_video_through_desktop_service_client() {
    let app_data_dir = PathBuf::from(required("ZHIHUA_TEST_APP_DATA_DIR"));
    let output_dir = PathBuf::from(required("ZHIHUA_TEST_OUTPUT_DIR"));
    std::fs::create_dir_all(&output_dir).expect("create validation output directory");

    let tunnel = SshTunnelManager::new(app_data_dir.clone()).expect("initialize SSH tunnel");
    let tunnel_status = tunnel.start().await.expect("start SSH tunnel");
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
    tunnel.stop().await.expect("stop validation tunnel");
}
