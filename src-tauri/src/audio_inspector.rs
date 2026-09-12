use crate::generation::GenerationStorage;
use serde::{Deserialize, Serialize};
use std::{path::Path, process::Command};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const FRAME_SAMPLES: usize = 256;
const FRAME_MILLISECONDS: u64 = 16;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateAudioInspection {
    pub candidate_id: String,
    pub has_audio: bool,
    pub speech_detected: bool,
    pub voiced_duration_ms: u64,
    pub voiced_ratio: f64,
    pub peak_voice_probability: f32,
    pub smart_eligible: bool,
    pub detail: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectCandidateAudioInput {
    pub candidate_id: String,
}

pub fn inspect_candidate(
    generations: &GenerationStorage,
    input: InspectCandidateAudioInput,
) -> Result<CandidateAudioInspection, String> {
    let candidate = generations
        .find(&input.candidate_id)
        .map_err(|error| error.message)?;
    inspect_audio_path(candidate.id, &candidate.local_path)
}

pub(crate) fn inspect_audio_path(
    candidate_id: String,
    path: &Path,
) -> Result<CandidateAudioInspection, String> {
    let output = command_output(
        "ffmpeg",
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            path_text(path)?,
            "-map",
            "0:a:0?",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-f",
            "s16le",
            "-",
        ],
    )?;
    if !output.status.success() || output.stdout.len() < FRAME_SAMPLES * 2 {
        return Ok(CandidateAudioInspection {
            candidate_id,
            has_audio: false,
            speech_detected: false,
            voiced_duration_ms: 0,
            voiced_ratio: 0.0,
            peak_voice_probability: 0.0,
            smart_eligible: false,
            detail: "候选没有可分析的原生音轨，导出会自动使用静音。".to_owned(),
        });
    }

    let samples = output
        .stdout
        .chunks_exact(2)
        .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
        .collect::<Vec<_>>();
    let mut detector = earshot::Detector::default();
    let mut voiced_frames = 0u64;
    let mut total_frames = 0u64;
    let mut peak = 0.0f32;
    for frame in samples.chunks_exact(FRAME_SAMPLES) {
        let score = detector.predict_i16(frame);
        peak = peak.max(score);
        if score >= 0.5 {
            voiced_frames += 1;
        }
        total_frames += 1;
    }
    let voiced_duration_ms = voiced_frames * FRAME_MILLISECONDS;
    let voiced_ratio = if total_frames == 0 {
        0.0
    } else {
        voiced_frames as f64 / total_frames as f64
    };
    // Short isolated transients are common in thunder, impacts and machinery.
    let speech_detected = voiced_duration_ms >= 480 && voiced_ratio >= 0.08;
    Ok(CandidateAudioInspection {
        candidate_id,
        has_audio: true,
        speech_detected,
        voiced_duration_ms,
        voiced_ratio,
        peak_voice_probability: peak,
        smart_eligible: true,
        detail: if speech_detected {
            "检测到持续的类人声声学活动；雷声、机械声也可能触发，请试听后决定是否保留。".to_owned()
        } else {
            "未检测到持续人声；智能使用可将环境音以低音量混入成片。".to_owned()
        },
    })
}

fn path_text(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "音频检查暂不支持该文件路径。".to_owned())
}

fn command_output(program: &str, args: &[&str]) -> Result<std::process::Output, String> {
    let mut command = Command::new(program);
    command.args(args);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
        .output()
        .map_err(|error| format!("无法启动音频检查工具：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_not_smart_eligible() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("empty.mp4");
        std::fs::write(&path, []).expect("empty fixture");
        let inspection = inspect_audio_path("candidate".to_owned(), &path)
            .expect("empty media produces an inspection result");
        assert!(!inspection.has_audio);
        assert!(!inspection.smart_eligible);
    }

    #[test]
    #[ignore = "requires the authorized H3 ambience and voiceover validation samples"]
    fn reports_voice_activity_without_discarding_authorized_h3_audio() {
        let ambience =
            std::env::var("ZHIHUA_TEST_H3_AMBIENCE_PATH").expect("ZHIHUA_TEST_H3_AMBIENCE_PATH");
        let voiceover =
            std::env::var("ZHIHUA_TEST_H3_VOICEOVER_PATH").expect("ZHIHUA_TEST_H3_VOICEOVER_PATH");
        let ambience = inspect_audio_path("ambience".to_owned(), Path::new(&ambience))
            .expect("ambience inspection");
        let voiceover = inspect_audio_path("voiceover".to_owned(), Path::new(&voiceover))
            .expect("voiceover inspection");
        println!("ambience: {ambience:?}");
        println!("voiceover: {voiceover:?}");
        assert!(ambience.has_audio);
        assert!(ambience.smart_eligible, "{}", ambience.detail);
        assert!(voiceover.has_audio);
        assert!(voiceover.speech_detected, "{}", voiceover.detail);
        assert!(voiceover.smart_eligible, "{}", voiceover.detail);
    }
}
