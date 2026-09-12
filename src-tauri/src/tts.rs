use crate::{asset::AssetStorage, storage::ProjectStorage, storyboard::StoryboardStorage};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, fs, path::PathBuf, process::Command, time::Duration};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsError {
    pub code: String,
    pub message: String,
}

impl TtsError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for TtsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

type TtsResult<T> = Result<T, TtsError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemVoice {
    pub id: String,
    pub name: String,
    pub locale: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SynthesizeNarrationInput {
    pub project_id: String,
    pub scene_id: String,
    pub voice_id: String,
    pub rate: i32,
    pub volume: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportNarrationInput {
    pub project_id: String,
    pub scene_id: String,
    pub asset_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrationArtifact {
    pub project_id: String,
    pub scene_id: String,
    pub voice_id: String,
    pub local_path: PathBuf,
    pub duration_ms: u64,
    pub size_bytes: u64,
    pub sha256: String,
    pub text_sha256: String,
    pub updated_at: String,
}

#[derive(Clone)]
pub struct SystemTtsProvider {
    projects: ProjectStorage,
    database_path: PathBuf,
}

impl SystemTtsProvider {
    pub fn initialize(projects: ProjectStorage) -> TtsResult<Self> {
        let provider = Self {
            database_path: projects.info().database_path,
            projects,
        };
        provider
            .connection()?
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS narration_artifacts (
                scene_id     TEXT PRIMARY KEY NOT NULL,
                project_id   TEXT NOT NULL,
                voice_id     TEXT NOT NULL,
                local_path   TEXT NOT NULL,
                duration_ms  INTEGER NOT NULL,
                size_bytes   INTEGER NOT NULL,
                sha256       TEXT NOT NULL,
                text_sha256  TEXT NOT NULL DEFAULT '',
                updated_at   TEXT NOT NULL,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE,
                FOREIGN KEY(scene_id) REFERENCES storyboard_scenes(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_narration_artifacts_project
                ON narration_artifacts(project_id, scene_id);
            ",
            )
            .map_err(database_error)?;
        let has_text_hash = provider
            .connection()?
            .prepare("PRAGMA table_info(narration_artifacts)")
            .map_err(database_error)?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?
            .iter()
            .any(|column| column == "text_sha256");
        if !has_text_hash {
            provider
                .connection()?
                .execute(
                    "ALTER TABLE narration_artifacts ADD COLUMN text_sha256 TEXT NOT NULL DEFAULT ''",
                    [],
                )
                .map_err(database_error)?;
        }
        Ok(provider)
    }

    fn connection(&self) -> TtsResult<Connection> {
        let connection = Connection::open(&self.database_path).map_err(database_error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(database_error)?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(database_error)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(database_error)?;
        Ok(connection)
    }

    pub fn list_voices(&self) -> TtsResult<Vec<SystemVoice>> {
        let script = r#"
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Speech
$speaker=New-Object System.Speech.Synthesis.SpeechSynthesizer
$speaker.GetInstalledVoices() | ForEach-Object {
  [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($_.VoiceInfo.Name)) + "`t" + $_.VoiceInfo.Culture.Name
}
$speaker.Dispose()
"#;
        let output = powershell(script, &[])?;
        let mut voices = Vec::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Some((encoded_name, locale)) = line.trim().split_once('\t') else {
                continue;
            };
            let name = String::from_utf8(
                STANDARD
                    .decode(encoded_name)
                    .map_err(|_| TtsError::new("VOICE_PARSE_ERROR", "系统语音名称无法解析"))?,
            )
            .map_err(|_| TtsError::new("VOICE_PARSE_ERROR", "系统语音名称不是有效文本"))?;
            voices.push(SystemVoice {
                id: name.clone(),
                name,
                locale: locale.to_owned(),
            });
        }
        if voices.is_empty() {
            return Err(TtsError::new(
                "NO_SYSTEM_VOICES",
                "Windows 没有可用的系统语音",
            ));
        }
        Ok(voices)
    }

    pub fn get_artifact(
        &self,
        project_id: &str,
        scene_id: &str,
    ) -> TtsResult<Option<NarrationArtifact>> {
        self.connection()?.query_row(
            "SELECT project_id, scene_id, voice_id, local_path, duration_ms, size_bytes, sha256, text_sha256, updated_at
             FROM narration_artifacts WHERE project_id = ?1 AND scene_id = ?2",
            params![project_id, scene_id],
            narration_from_row,
        ).optional().map_err(database_error)
    }

    pub fn synthesize(
        &self,
        storyboard: &StoryboardStorage,
        input: SynthesizeNarrationInput,
    ) -> TtsResult<NarrationArtifact> {
        if !(-10..=10).contains(&input.rate) || input.volume > 100 {
            return Err(TtsError::new(
                "INVALID_TTS_SETTINGS",
                "语速或音量超出允许范围",
            ));
        }
        let voice = self
            .list_voices()?
            .into_iter()
            .find(|voice| voice.id == input.voice_id)
            .ok_or_else(|| TtsError::new("VOICE_NOT_FOUND", "所选系统语音不存在"))?;
        let scene = storyboard
            .list(&input.project_id)
            .map_err(|error| TtsError::new("STORYBOARD_ERROR", error.to_string()))?
            .into_iter()
            .find(|scene| scene.id == input.scene_id)
            .ok_or_else(|| TtsError::new("SCENE_NOT_FOUND", "分镜不存在"))?;
        if scene.narration.trim().is_empty() {
            return Err(TtsError::new("EMPTY_NARRATION", "请先填写旁白文案"));
        }
        let project = self
            .projects
            .get_project(&input.project_id)
            .map_err(|error| TtsError::new("PROJECT_ERROR", error.to_string()))?;
        let directory = project.project_dir.join("audio").join("narration");
        fs::create_dir_all(&directory).map_err(io_error)?;
        let safe_id = format!("{:x}", Sha256::digest(input.scene_id.as_bytes()));
        let output_path = directory.join(format!("{safe_id}.wav"));
        let temporary_path = directory.join(format!("{safe_id}.wav.tmp"));
        let text_path = directory.join(format!("{safe_id}.txt.tmp"));
        let _ = fs::remove_file(&temporary_path);
        fs::write(&text_path, scene.narration.as_bytes()).map_err(io_error)?;
        let script = r#"
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Speech
$text=[IO.File]::ReadAllText($env:ZHIHUA_TTS_TEXT,[Text.Encoding]::UTF8)
$speaker=New-Object System.Speech.Synthesis.SpeechSynthesizer
$speaker.SelectVoice($env:ZHIHUA_TTS_VOICE)
$speaker.Rate=[int]$env:ZHIHUA_TTS_RATE
$speaker.Volume=[int]$env:ZHIHUA_TTS_VOLUME
$speaker.SetOutputToWaveFile($env:ZHIHUA_TTS_OUTPUT)
$speaker.Speak($text)
$speaker.Dispose()
"#;
        let rate_text = input.rate.to_string();
        let volume_text = input.volume.to_string();
        let result = powershell(
            script,
            &[
                ("ZHIHUA_TTS_TEXT", text_path.as_os_str()),
                ("ZHIHUA_TTS_VOICE", std::ffi::OsStr::new(&voice.id)),
                ("ZHIHUA_TTS_RATE", std::ffi::OsStr::new(&rate_text)),
                ("ZHIHUA_TTS_VOLUME", std::ffi::OsStr::new(&volume_text)),
                ("ZHIHUA_TTS_OUTPUT", temporary_path.as_os_str()),
            ],
        );
        let _ = fs::remove_file(&text_path);
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary_path);
            return Err(error);
        }
        if !temporary_path.is_file() {
            return Err(TtsError::new(
                "TTS_OUTPUT_MISSING",
                "系统语音没有生成音频文件",
            ));
        }
        if output_path.exists() {
            fs::remove_file(&output_path).map_err(io_error)?;
        }
        fs::rename(&temporary_path, &output_path).map_err(io_error)?;
        let bytes = fs::read(&output_path).map_err(io_error)?;
        let artifact = NarrationArtifact {
            project_id: input.project_id,
            scene_id: input.scene_id,
            voice_id: voice.id,
            duration_ms: wav_duration_ms(&bytes)?,
            size_bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            text_sha256: narration_text_sha256(&scene.narration),
            local_path: output_path,
            updated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        };
        self.save_artifact(&artifact)?;
        Ok(artifact)
    }

    pub fn import_audio(
        &self,
        storyboard: &StoryboardStorage,
        assets: &AssetStorage,
        input: ImportNarrationInput,
    ) -> TtsResult<NarrationArtifact> {
        let scene = storyboard
            .list(&input.project_id)
            .map_err(|error| TtsError::new("STORYBOARD_ERROR", error.to_string()))?
            .into_iter()
            .find(|scene| scene.id == input.scene_id)
            .ok_or_else(|| TtsError::new("SCENE_NOT_FOUND", "分镜不存在"))?;
        if scene.narration.trim().is_empty() {
            return Err(TtsError::new(
                "EMPTY_NARRATION",
                "请先填写与录音对应的旁白文案",
            ));
        }
        let asset = assets
            .list(&input.project_id)
            .map_err(|error| TtsError::new("ASSET_ERROR", error.message))?
            .into_iter()
            .find(|asset| asset.id == input.asset_id && asset.media_type == "audio")
            .ok_or_else(|| TtsError::new("AUDIO_ASSET_NOT_FOUND", "所选素材不存在或不是音频"))?;
        let version = asset
            .versions
            .into_iter()
            .find(|version| version.id == asset.current_version_id)
            .ok_or_else(|| TtsError::new("AUDIO_ASSET_NOT_FOUND", "所选录音版本不存在"))?;
        if !version.stored_path.is_file() {
            return Err(TtsError::new("AUDIO_FILE_MISSING", "所选录音文件已丢失"));
        }
        let project = self
            .projects
            .get_project(&input.project_id)
            .map_err(|error| TtsError::new("PROJECT_ERROR", error.to_string()))?;
        let directory = project.project_dir.join("audio").join("narration");
        fs::create_dir_all(&directory).map_err(io_error)?;
        let safe_id = format!("{:x}", Sha256::digest(input.scene_id.as_bytes()));
        let output_path = directory.join(format!("{safe_id}.wav"));
        let temporary_path = directory.join(format!("{safe_id}.import.tmp"));
        let _ = fs::remove_file(&temporary_path);
        let mut command = Command::new("ffmpeg");
        command
            .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(&version.stored_path)
            .args([
                "-vn",
                "-ac",
                "1",
                "-ar",
                "24000",
                "-c:a",
                "pcm_s16le",
                "-f",
                "wav",
            ])
            .arg(&temporary_path);
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);
        let output = command.output().map_err(|error| {
            TtsError::new(
                "FFMPEG_UNAVAILABLE",
                format!("无法启动 FFmpeg 处理旁白录音：{error}"),
            )
        })?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(TtsError::new(
                "AUDIO_IMPORT_FAILED",
                if detail.is_empty() {
                    "旁白录音无法转换为 WAV".to_owned()
                } else {
                    detail
                },
            ));
        }
        if output_path.exists() {
            fs::remove_file(&output_path).map_err(io_error)?;
        }
        fs::rename(&temporary_path, &output_path).map_err(io_error)?;
        let bytes = fs::read(&output_path).map_err(io_error)?;
        let artifact = NarrationArtifact {
            project_id: input.project_id,
            scene_id: input.scene_id,
            voice_id: format!("imported:{}", input.asset_id),
            local_path: output_path,
            duration_ms: wav_duration_ms(&bytes)?,
            size_bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            text_sha256: narration_text_sha256(&scene.narration),
            updated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        };
        self.save_artifact(&artifact)?;
        Ok(artifact)
    }

    fn save_artifact(&self, artifact: &NarrationArtifact) -> TtsResult<()> {
        self.connection()?.execute(
            "INSERT INTO narration_artifacts
             (scene_id, project_id, voice_id, local_path, duration_ms, size_bytes, sha256, text_sha256, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(scene_id) DO UPDATE SET voice_id=excluded.voice_id, local_path=excluded.local_path,
             duration_ms=excluded.duration_ms, size_bytes=excluded.size_bytes, sha256=excluded.sha256,
             text_sha256=excluded.text_sha256, updated_at=excluded.updated_at
             WHERE narration_artifacts.project_id=excluded.project_id",
            params![artifact.scene_id, artifact.project_id, artifact.voice_id,
                artifact.local_path.to_string_lossy(), artifact.duration_ms, artifact.size_bytes,
                artifact.sha256, artifact.text_sha256, artifact.updated_at],
        ).map_err(database_error)?;
        Ok(())
    }
}

fn narration_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<NarrationArtifact> {
    Ok(NarrationArtifact {
        project_id: row.get(0)?,
        scene_id: row.get(1)?,
        voice_id: row.get(2)?,
        local_path: PathBuf::from(row.get::<_, String>(3)?),
        duration_ms: row.get(4)?,
        size_bytes: row.get(5)?,
        sha256: row.get(6)?,
        text_sha256: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

pub(crate) fn narration_text_sha256(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn powershell(
    script: &str,
    environment: &[(&str, &std::ffi::OsStr)],
) -> TtsResult<std::process::Output> {
    let encoded = STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
        &encoded,
    ]);
    for (key, value) in environment {
        command.env(key, value);
    }
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command.output().map_err(|error| {
        TtsError::new(
            "TTS_PROCESS_ERROR",
            format!("无法启动 Windows 系统语音：{error}"),
        )
    })?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(TtsError::new(
            "TTS_PROCESS_ERROR",
            if detail.is_empty() {
                "Windows 系统语音执行失败".to_owned()
            } else {
                detail
            },
        ));
    }
    Ok(output)
}

fn wav_duration_ms(bytes: &[u8]) -> TtsResult<u64> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(TtsError::new(
            "INVALID_WAV",
            "系统语音输出不是有效 WAV 文件",
        ));
    }
    let mut offset = 12usize;
    let mut byte_rate = None;
    let mut data_size = None;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let start = offset + 8;
        if start + size > bytes.len() {
            break;
        }
        if id == b"fmt " && size >= 12 {
            byte_rate =
                Some(u32::from_le_bytes(bytes[start + 8..start + 12].try_into().unwrap()) as u64);
        } else if id == b"data" {
            data_size = Some(size as u64);
        }
        offset = start + size + (size % 2);
    }
    match (byte_rate, data_size) {
        (Some(rate), Some(size)) if rate > 0 => Ok(size.saturating_mul(1000) / rate),
        _ => Err(TtsError::new("INVALID_WAV", "无法读取系统语音 WAV 时长")),
    }
}

fn database_error(error: rusqlite::Error) -> TtsError {
    TtsError::new("DATABASE_ERROR", format!("旁白数据库操作失败：{error}"))
}
fn io_error(error: std::io::Error) -> TtsError {
    TtsError::new("TTS_IO_ERROR", format!("旁白文件操作失败：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        storage::CreateProjectInput,
        storyboard::{
            CandidateQuality, GenerationMode, SceneDraft, SceneStatus, StoryboardStorage,
        },
    };

    #[test]
    fn reads_pcm_wav_duration() {
        let mut bytes = vec![0u8; 44 + 32_000];
        bytes[0..4].copy_from_slice(b"RIFF");
        bytes[8..12].copy_from_slice(b"WAVE");
        bytes[12..16].copy_from_slice(b"fmt ");
        bytes[16..20].copy_from_slice(&16u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&32_000u32.to_le_bytes());
        bytes[36..40].copy_from_slice(b"data");
        bytes[40..44].copy_from_slice(&32_000u32.to_le_bytes());
        assert_eq!(wav_duration_ms(&bytes).expect("duration"), 1_000);
    }

    #[test]
    #[ignore = "requires a Windows system voice"]
    fn synthesizes_a_real_windows_voice_file() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let projects = ProjectStorage::initialize(
            directory.path().join("zhihua.sqlite3"),
            directory.path().join("projects"),
        )
        .expect("initialize projects");
        let project = projects
            .create_project(CreateProjectInput {
                title: "旁白测试".to_owned(),
                audience: None,
                target_duration_sec: Some(5),
            })
            .expect("create project");
        let storyboard = StoryboardStorage::initialize(projects.clone()).expect("storyboard");
        let scene = storyboard
            .upsert(SceneDraft {
                id: "scene-tts".to_owned(),
                project_id: project.id.clone(),
                order: 0,
                title: "测试".to_owned(),
                purpose: "验证系统旁白".to_owned(),
                source_refs: Vec::new(),
                narration: "闪电发生后，我们会听到雷声。".to_owned(),
                narration_mode: crate::storyboard::NarrationMode::Tts,
                ambient_sound: "雨声和雷声".to_owned(),
                on_screen_text: Vec::new(),
                visual_plan: "雷雨云".to_owned(),
                visual_intent: crate::storyboard::VisualIntent::default(),
                prompt_mode: crate::storyboard::PromptMode::Quick,
                audio_intent: crate::storyboard::AudioIntent::Environment,
                locked: false,
                generation_mode: GenerationMode::T2v,
                target_duration_ms: 5_000,
                asset_ids: Vec::new(),
                selected_version_id: None,
                last_job_id: None,
                last_upscale_job_id: None,
                pending_request_id: None,
                generation_stage: None,
                status: SceneStatus::Draft,
                quality: CandidateQuality::Fast,
                updated_at: String::new(),
            })
            .expect("scene");
        let provider = SystemTtsProvider::initialize(projects).expect("tts provider");
        let voice = provider
            .list_voices()
            .expect("list voices")
            .into_iter()
            .find(|voice| voice.locale.starts_with("zh"))
            .expect("Chinese system voice");
        let artifact = provider
            .synthesize(
                &storyboard,
                SynthesizeNarrationInput {
                    project_id: project.id.clone(),
                    scene_id: scene.id.clone(),
                    voice_id: voice.id,
                    rate: 0,
                    volume: 100,
                },
            )
            .expect("synthesize");
        assert!(artifact.local_path.is_file());
        assert!(artifact.duration_ms > 500);
        assert_eq!(
            provider
                .get_artifact(&project.id, &scene.id)
                .expect("artifact")
                .expect("stored artifact")
                .sha256,
            artifact.sha256
        );
    }
}
