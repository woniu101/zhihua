use crate::{
    asset::AssetStorage,
    frame_profile::{FrameAspectRatio, FrameSize},
    generation::GenerationStorage,
    storage::ProjectStorage,
    storyboard::{NarrationMode, StoryboardStorage},
    tts::{narration_text_sha256, SystemTtsProvider},
};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt, fs,
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportError {
    pub code: String,
    pub message: String,
}

impl ExportError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

type ExportResult<T> = Result<T, ExportError>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportCapability {
    pub available: bool,
    pub label: String,
    pub reason: String,
    pub ffmpeg_path: Option<PathBuf>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubtitleMode {
    BurnAndSrt,
    Burn,
    Srt,
    None,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvironmentAudioPolicy {
    Smart,
    Always,
    Off,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputRendition {
    Candidate,
    Enhanced1080p,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProjectInput {
    pub project_id: String,
    pub output_directory: PathBuf,
    pub frame_rate: u32,
    pub subtitle_mode: SubtitleMode,
    pub aspect_ratio: FrameAspectRatio,
    pub rendition: OutputRendition,
    pub narration_volume: u32,
    pub music_asset_id: Option<String>,
    pub music_volume: u32,
    pub music_fade: bool,
    pub environment_audio_policy: EnvironmentAudioPolicy,
    pub environment_volume: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewProjectInput {
    pub project_id: String,
    pub frame_rate: u32,
    pub aspect_ratio: FrameAspectRatio,
    pub rendition: OutputRendition,
    pub subtitle_mode: SubtitleMode,
    pub narration_volume: u32,
    pub music_asset_id: Option<String>,
    pub music_volume: u32,
    pub music_fade: bool,
    pub environment_audio_policy: EnvironmentAudioPolicy,
    pub environment_volume: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectExport {
    pub output_path: PathBuf,
    pub subtitle_path: Option<PathBuf>,
    pub duration_ms: u64,
    pub size_bytes: u64,
    pub sha256: String,
    pub created_at: String,
    pub width: u32,
    pub height: u32,
    pub enhanced_scene_count: u32,
    pub scaled_scene_count: u32,
}

#[derive(Clone)]
pub struct FfmpegExporter {
    projects: ProjectStorage,
}

struct ExportScene {
    narration: String,
    duration_ms: u32,
    video_path: PathBuf,
    narration_path: Option<PathBuf>,
    include_environment: bool,
    uses_enhanced_source: bool,
}

impl FfmpegExporter {
    pub fn new(projects: ProjectStorage) -> Self {
        Self { projects }
    }

    pub fn capability(&self) -> ExportCapability {
        match command_output("ffmpeg", &["-version"]) {
            Ok(output) if output.status.success() => {
                let first = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("FFmpeg")
                    .trim()
                    .to_owned();
                ExportCapability {
                    available: true,
                    label: "FFmpeg 可用".to_owned(),
                    reason: "可以在本机合成 H.264 + AAC MP4。".to_owned(),
                    ffmpeg_path: find_executable("ffmpeg"),
                    version: Some(first),
                }
            }
            Ok(output) => ExportCapability {
                available: false,
                label: "FFmpeg 不可用".to_owned(),
                reason: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                ffmpeg_path: find_executable("ffmpeg"),
                version: None,
            },
            Err(error) => ExportCapability {
                available: false,
                label: "FFmpeg 不可用".to_owned(),
                reason: error.message,
                ffmpeg_path: None,
                version: None,
            },
        }
    }

    pub fn preview(
        &self,
        storyboards: &StoryboardStorage,
        generations: &GenerationStorage,
        assets: &AssetStorage,
        tts: &SystemTtsProvider,
        input: PreviewProjectInput,
    ) -> ExportResult<ProjectExport> {
        let project = self
            .projects
            .get_project(&input.project_id)
            .map_err(|error| ExportError::new("PROJECT_ERROR", error.to_string()))?;
        let output_directory = project.project_dir.join("previews");
        fs::create_dir_all(&output_directory).map_err(io_error)?;
        if let Ok(entries) = fs::read_dir(&output_directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file()
                    && matches!(
                        path.extension().and_then(|value| value.to_str()),
                        Some("mp4" | "srt")
                    )
                {
                    let _ = fs::remove_file(path);
                }
            }
        }
        self.export(
            storyboards,
            generations,
            assets,
            tts,
            ExportProjectInput {
                project_id: input.project_id,
                output_directory,
                frame_rate: input.frame_rate,
                subtitle_mode: input.subtitle_mode,
                aspect_ratio: input.aspect_ratio,
                rendition: input.rendition,
                narration_volume: input.narration_volume,
                music_asset_id: input.music_asset_id,
                music_volume: input.music_volume,
                music_fade: input.music_fade,
                environment_audio_policy: input.environment_audio_policy,
                environment_volume: input.environment_volume,
            },
        )
    }

    pub fn export(
        &self,
        storyboards: &StoryboardStorage,
        generations: &GenerationStorage,
        assets: &AssetStorage,
        tts: &SystemTtsProvider,
        input: ExportProjectInput,
    ) -> ExportResult<ProjectExport> {
        if !matches!(input.frame_rate, 24 | 25 | 30)
            || input.narration_volume > 100
            || input.music_volume > 100
            || input.environment_volume > 100
        {
            return Err(ExportError::new(
                "INVALID_EXPORT_SETTINGS",
                "帧率或旁白音量无效",
            ));
        }
        let project = self
            .projects
            .get_project(&input.project_id)
            .map_err(|error| ExportError::new("PROJECT_ERROR", error.to_string()))?;
        let scenes = storyboards
            .list(&input.project_id)
            .map_err(|error| ExportError::new("STORYBOARD_ERROR", error.to_string()))?;
        if scenes.is_empty() {
            return Err(ExportError::new("NO_SCENES", "当前项目没有分镜"));
        }
        let mut export_scenes = Vec::with_capacity(scenes.len());
        for scene in &scenes {
            let selected_id = scene.selected_version_id.as_deref().ok_or_else(|| {
                ExportError::new(
                    "MISSING_OFFICIAL_VERSION",
                    format!("分镜“{}”尚未选择正式版本", scene.title),
                )
            })?;
            let candidate = generations
                .list(&input.project_id, &scene.id)
                .map_err(|error| ExportError::new("GENERATION_ERROR", error.message))?
                .into_iter()
                .find(|item| item.id == selected_id)
                .ok_or_else(|| {
                    ExportError::new(
                        "MISSING_OFFICIAL_VERSION",
                        format!("分镜“{}”选择的正式版本不存在", scene.title),
                    )
                })?;
            let candidate_dimensions = input.aspect_ratio.profile().visible;
            if candidate.aspect_ratio != input.aspect_ratio.label()
                || candidate.visible_width != candidate_dimensions.width
                || candidate.visible_height != candidate_dimensions.height
            {
                return Err(ExportError::new(
                    "CANDIDATE_FRAME_MISMATCH",
                    format!(
                        "分镜“{}”的正式版本画幅与当前项目不一致，请按 {} 重新生成或重新选择",
                        scene.title,
                        input.aspect_ratio.label()
                    ),
                ));
            }
            let enhanced = if matches!(input.rendition, OutputRendition::Enhanced1080p) {
                generations
                    .list_enhanced(&input.project_id, &scene.id)
                    .map_err(|error| ExportError::new("GENERATION_ERROR", error.message))?
                    .into_iter()
                    .rev()
                    .find(|item| {
                        item.source_candidate_id == selected_id && item.local_path.is_file()
                    })
            } else {
                None
            };
            let (video_path, uses_enhanced_source) = enhanced
                .map(|item| (item.local_path, true))
                .unwrap_or((candidate.local_path, false));
            if !video_path.is_file() {
                return Err(ExportError::new(
                    "MISSING_VIDEO_FILE",
                    format!("分镜“{}”的正式版本文件已丢失", scene.title),
                ));
            }
            let narration_path = if input.narration_volume == 0
                || matches!(scene.narration_mode, NarrationMode::None)
            {
                None
            } else {
                if scene.narration.trim().is_empty() {
                    return Err(ExportError::new(
                        "MISSING_NARRATION_TEXT",
                        format!("分镜“{}”已启用旁白，但旁白文案为空", scene.title),
                    ));
                }
                let narration = tts
                    .get_artifact(&input.project_id, &scene.id)
                    .map_err(|error| ExportError::new("TTS_ERROR", error.message))?
                    .ok_or_else(|| {
                        ExportError::new(
                            "MISSING_NARRATION",
                            format!("分镜“{}”尚未生成或导入旁白", scene.title),
                        )
                    })?;
                if !narration.local_path.is_file() {
                    return Err(ExportError::new(
                        "MISSING_NARRATION_FILE",
                        format!("分镜“{}”的旁白文件已丢失", scene.title),
                    ));
                }
                if narration.text_sha256 != narration_text_sha256(&scene.narration) {
                    return Err(ExportError::new(
                        "STALE_NARRATION",
                        format!("分镜“{}”的旁白文案已修改，请重新生成旁白", scene.title),
                    ));
                }
                if narration.duration_ms > scene.target_duration_ms as u64 + 250 {
                    return Err(ExportError::new(
                        "NARRATION_TOO_LONG",
                        format!(
                            "分镜“{}”的旁白为 {:.1} 秒，超过镜头时长 {:.1} 秒，请精简文案或延长镜头",
                            scene.title,
                            narration.duration_ms as f64 / 1000.0,
                            scene.target_duration_ms as f64 / 1000.0
                        ),
                    ));
                }
                Some(narration.local_path)
            };
            let include_environment = match input.environment_audio_policy {
                EnvironmentAudioPolicy::Off => false,
                EnvironmentAudioPolicy::Always => has_audio_stream(&video_path),
                EnvironmentAudioPolicy::Smart => {
                    crate::audio_inspector::inspect_audio_path(candidate.id.clone(), &video_path)
                        .map(|inspection| inspection.smart_eligible)
                        .unwrap_or(false)
                }
            };
            export_scenes.push(ExportScene {
                narration: if matches!(scene.narration_mode, NarrationMode::None) {
                    String::new()
                } else {
                    scene.narration.clone()
                },
                duration_ms: scene.target_duration_ms,
                video_path,
                narration_path,
                include_environment,
                uses_enhanced_source,
            });
        }
        let music_path = input
            .music_asset_id
            .as_deref()
            .map(|asset_id| {
                let asset = assets
                    .list(&input.project_id)
                    .map_err(|error| ExportError::new("ASSET_ERROR", error.message))?
                    .into_iter()
                    .find(|asset| asset.id == asset_id && asset.media_type == "audio")
                    .ok_or_else(|| {
                        ExportError::new("MUSIC_ASSET_NOT_FOUND", "所选背景音乐不存在或不是音频")
                    })?;
                let version = asset
                    .versions
                    .into_iter()
                    .find(|version| version.id == asset.current_version_id)
                    .ok_or_else(|| ExportError::new("MUSIC_FILE_MISSING", "背景音乐版本不存在"))?;
                if !version.stored_path.is_file() {
                    return Err(ExportError::new("MUSIC_FILE_MISSING", "背景音乐文件已丢失"));
                }
                Ok(version.stored_path)
            })
            .transpose()?;
        let output_directory = if input.output_directory.as_os_str().is_empty() {
            project.project_dir.join("exports")
        } else {
            if !input.output_directory.is_absolute() {
                return Err(ExportError::new(
                    "INVALID_OUTPUT_DIRECTORY",
                    "导出目录必须是绝对路径",
                ));
            }
            input.output_directory.clone()
        };
        fs::create_dir_all(&output_directory).map_err(io_error)?;
        let workspace_root = project.project_dir.join("exports");
        fs::create_dir_all(&workspace_root).map_err(io_error)?;
        let workspace = workspace_root.join(format!(".work-{}", Uuid::new_v4()));
        fs::create_dir(&workspace).map_err(io_error)?;
        let result = self.render(
            &workspace,
            &output_directory,
            &project.title,
            &export_scenes,
            music_path.as_deref(),
            &input,
        );
        let _ = fs::remove_dir_all(&workspace);
        result
    }

    fn render(
        &self,
        workspace: &Path,
        output_directory: &Path,
        project_title: &str,
        scenes: &[ExportScene],
        music_path: Option<&Path>,
        input: &ExportProjectInput,
    ) -> ExportResult<ProjectExport> {
        let mut normalized = Vec::with_capacity(scenes.len());
        let profile = input.aspect_ratio.profile();
        let dimensions: FrameSize = if matches!(input.rendition, OutputRendition::Enhanced1080p) {
            profile.full_hd
        } else {
            profile.visible
        };
        for (index, scene) in scenes.iter().enumerate() {
            let path = workspace.join(format!("scene-{index:03}.mp4"));
            let duration = format!("{:.3}", scene.duration_ms as f64 / 1000.0);
            let narration_volume = input.narration_volume as f64 / 100.0;
            let environment_volume = input.environment_volume as f64 / 100.0;
            let frame_rate = input.frame_rate.to_string();
            let video_filter = format!(
                "[0:v]scale={}:{}:flags=lanczos,setsar=1,fps={frame_rate},tpad=stop_mode=clone:stop_duration=15,trim=duration={duration}[v]",
                dimensions.width,
                dimensions.height,
            );
            let filters = match (scene.include_environment, scene.narration_path.is_some()) {
                (true, true) => format!(
                    "{video_filter};[0:a]volume={environment_volume:.2},apad,atrim=0:{duration}[env];[1:a]volume={narration_volume:.2},apad,atrim=0:{duration},asplit=2[voice][side];[env][side]sidechaincompress=threshold=0.03:ratio=8:attack=20:release=400[ducked];[ducked][voice]amix=inputs=2:normalize=0:duration=longest[a]"
                ),
                (true, false) => format!(
                    "{video_filter};[0:a]volume={environment_volume:.2},apad,atrim=0:{duration}[a]"
                ),
                (false, true) => format!(
                    "{video_filter};[1:a]volume={narration_volume:.2},apad,atrim=0:{duration}[a]"
                ),
                (false, false) => format!(
                    "{video_filter};anullsrc=r=48000:cl=stereo,atrim=0:{duration}[a]"
                ),
            };
            let mut args = vec![
                "-y".to_owned(),
                "-hide_banner".to_owned(),
                "-loglevel".to_owned(),
                "error".to_owned(),
                "-i".to_owned(),
                path_text(&scene.video_path)?.to_owned(),
            ];
            if let Some(narration_path) = &scene.narration_path {
                args.push("-i".to_owned());
                args.push(path_text(narration_path)?.to_owned());
            }
            args.extend([
                "-filter_complex".to_owned(),
                filters,
                "-map".to_owned(),
                "[v]".to_owned(),
                "-map".to_owned(),
                "[a]".to_owned(),
                "-t".to_owned(),
                duration.clone(),
                "-c:v".to_owned(),
                "libx264".to_owned(),
                "-preset".to_owned(),
                "medium".to_owned(),
                "-crf".to_owned(),
                "18".to_owned(),
                "-pix_fmt".to_owned(),
                "yuv420p".to_owned(),
                "-c:a".to_owned(),
                "aac".to_owned(),
                "-b:a".to_owned(),
                "192k".to_owned(),
                "-ar".to_owned(),
                "48000".to_owned(),
                "-ac".to_owned(),
                "2".to_owned(),
                path_text(&path)?.to_owned(),
            ]);
            let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
            run_ffmpeg(&arg_refs)?;
            normalized.push(path);
        }
        let concat_path = workspace.join("concat.txt");
        let concat = normalized
            .iter()
            .map(|path| {
                let escaped = path
                    .to_string_lossy()
                    .replace('\\', "/")
                    .replace('\u{27}', "'\\''");
                format!("file '{escaped}'\n")
            })
            .collect::<String>();
        fs::write(&concat_path, concat).map_err(io_error)?;
        let base_path = workspace.join("combined.mp4");
        run_ffmpeg(&[
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            path_text(&concat_path)?,
            "-c",
            "copy",
            path_text(&base_path)?,
        ])?;

        let total_ms = scenes.iter().map(|scene| scene.duration_ms as u64).sum();
        let delivery_input = if let Some(music_path) = music_path {
            let mixed_path = workspace.join("mixed.mp4");
            let total_seconds = total_ms as f64 / 1000.0;
            let music_volume = input.music_volume as f64 / 100.0;
            let fade = if input.music_fade && total_seconds > 1.0 {
                format!(
                    ",afade=t=in:st=0:d=1,afade=t=out:st={:.3}:d=1",
                    (total_seconds - 1.0).max(0.0)
                )
            } else {
                String::new()
            };
            let mix_filter = format!(
                "[1:a]volume={music_volume:.2}{fade}[music];[0:a][music]amix=inputs=2:duration=first:dropout_transition=2[a]"
            );
            let duration = format!("{total_seconds:.3}");
            run_ffmpeg(&[
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                path_text(&base_path)?,
                "-stream_loop",
                "-1",
                "-i",
                path_text(music_path)?,
                "-filter_complex",
                &mix_filter,
                "-map",
                "0:v",
                "-map",
                "[a]",
                "-t",
                &duration,
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                path_text(&mixed_path)?,
            ])?;
            mixed_path
        } else {
            base_path
        };
        let subtitle_path =
            output_directory.join(format!("{}-{}.srt", safe_name(project_title), timestamp()));
        let subtitle_content = build_srt(scenes);
        let has_subtitles = !subtitle_content.trim().is_empty()
            && !matches!(input.subtitle_mode, SubtitleMode::None);
        if has_subtitles {
            fs::write(&subtitle_path, &subtitle_content).map_err(io_error)?;
        }
        let output_path = subtitle_path.with_extension("mp4");
        if has_subtitles
            && matches!(
                input.subtitle_mode,
                SubtitleMode::Burn | SubtitleMode::BurnAndSrt
            )
        {
            let filter_path = subtitle_path
                .to_string_lossy()
                .replace('\\', "/")
                .replace(':', "\\:")
                .replace('\u{27}', "\\'");
            run_ffmpeg(&[
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                path_text(&delivery_input)?,
                "-vf",
                &format!("subtitles='{filter_path}'"),
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "copy",
                path_text(&output_path)?,
            ])?;
        } else {
            fs::copy(&delivery_input, &output_path).map_err(io_error)?;
        }
        let visible_subtitle =
            if !has_subtitles || matches!(input.subtitle_mode, SubtitleMode::Burn) {
                let _ = fs::remove_file(&subtitle_path);
                None
            } else {
                Some(subtitle_path)
            };
        let bytes = fs::read(&output_path).map_err(io_error)?;
        Ok(ProjectExport {
            output_path,
            subtitle_path: visible_subtitle,
            duration_ms: total_ms,
            size_bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            width: dimensions.width,
            height: dimensions.height,
            enhanced_scene_count: scenes
                .iter()
                .filter(|scene| scene.uses_enhanced_source)
                .count() as u32,
            scaled_scene_count: scenes
                .iter()
                .filter(|scene| !scene.uses_enhanced_source)
                .count() as u32,
        })
    }
}

fn build_srt(scenes: &[ExportScene]) -> String {
    let mut scene_start = 0u64;
    let mut cue_index = 1usize;
    let mut output = String::new();
    for scene in scenes {
        let sentences = split_subtitle_sentences(&scene.narration);
        let scene_duration = scene.duration_ms as u64;
        let total_weight = sentences
            .iter()
            .map(|sentence| sentence.chars().count().max(1) as u64)
            .sum::<u64>();
        let mut cue_start = scene_start;
        for (index, sentence) in sentences.iter().enumerate() {
            let cue_end = if index + 1 == sentences.len() {
                scene_start + scene_duration
            } else {
                cue_start
                    + scene_duration * sentence.chars().count().max(1) as u64 / total_weight.max(1)
            };
            output.push_str(&format!(
                "{}\n{} --> {}\n{}\n\n",
                cue_index,
                srt_time(cue_start),
                srt_time(cue_end.max(cue_start + 1)),
                sentence.replace('<', "＜").replace('>', "＞")
            ));
            cue_index += 1;
            cue_start = cue_end;
        }
        scene_start += scene_duration;
    }
    output
}

fn split_subtitle_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    for character in text.replace(['\r', '\n'], " ").chars() {
        current.push(character);
        if matches!(character, '。' | '！' | '？' | '；' | '!' | '?' | ';') {
            let sentence = current.trim();
            if !sentence.is_empty() {
                sentences.push(sentence.to_owned());
            }
            current.clear();
        }
    }
    let remainder = current.trim();
    if !remainder.is_empty() {
        sentences.push(remainder.to_owned());
    }
    sentences
}

fn srt_time(milliseconds: u64) -> String {
    let hours = milliseconds / 3_600_000;
    let minutes = milliseconds / 60_000 % 60;
    let seconds = milliseconds / 1_000 % 60;
    let millis = milliseconds % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
}

fn has_audio_stream(path: &Path) -> bool {
    let Ok(path) = path_text(path) else {
        return false;
    };
    command_output(
        "ffprobe",
        &[
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=index",
            "-of",
            "csv=p=0",
            path,
        ],
    )
    .map(|output| output.status.success() && !output.stdout.is_empty())
    .unwrap_or(false)
}

fn run_ffmpeg(args: &[&str]) -> ExportResult<()> {
    let output = command_output("ffmpeg", args)?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let tail = detail
        .lines()
        .rev()
        .take(5)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("；");
    Err(ExportError::new(
        "FFMPEG_FAILED",
        if tail.is_empty() {
            "FFmpeg 合成失败".to_owned()
        } else {
            tail
        },
    ))
}

fn command_output(program: &str, args: &[&str]) -> ExportResult<std::process::Output> {
    let mut command = Command::new(program);
    command.args(args);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command.output().map_err(|error| {
        ExportError::new("FFMPEG_UNAVAILABLE", format!("无法启动 FFmpeg：{error}"))
    })
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(format!("{name}.exe")))
        .find(|path| path.is_file())
}

fn path_text(path: &Path) -> ExportResult<&str> {
    path.to_str()
        .ok_or_else(|| ExportError::new("INVALID_PATH", "FFmpeg 暂不支持该文件路径"))
}

fn safe_name(value: &str) -> String {
    let value = value
        .chars()
        .map(|character| {
            if matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let value = value.trim().trim_end_matches(['.', ' ']);
    if value.is_empty() {
        "知画成片".to_owned()
    } else {
        value.chars().take(60).collect()
    }
}

fn timestamp() -> String {
    Utc::now().format("%Y%m%d-%H%M%S").to_string()
}
fn io_error(error: std::io::Error) -> ExportError {
    ExportError::new("EXPORT_IO_ERROR", format!("导出文件操作失败：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_monotonic_srt_and_sanitizes_markup() {
        let scenes = vec![
            ExportScene {
                narration: "第一句。第二句！".into(),
                duration_ms: 5_000,
                video_path: "a".into(),
                narration_path: Some("b".into()),
                include_environment: false,
                uses_enhanced_source: false,
            },
            ExportScene {
                narration: "第二句 <b>".into(),
                duration_ms: 10_000,
                video_path: "c".into(),
                narration_path: Some("d".into()),
                include_environment: false,
                uses_enhanced_source: false,
            },
        ];
        let srt = build_srt(&scenes);
        assert!(srt.contains("1\n00:00:00,000 --> 00:00:02,500\n第一句。"));
        assert!(srt.contains("2\n00:00:02,500 --> 00:00:05,000\n第二句！"));
        assert!(srt.contains("3\n00:00:05,000 --> 00:00:15,000"));
        assert!(srt.contains("第二句 ＜b＞"));
    }

    #[test]
    fn produces_windows_safe_output_names() {
        assert_eq!(safe_name("雷电:为什么?/"), "雷电_为什么__");
    }

    #[test]
    #[ignore = "requires ffmpeg with libx264, aac and subtitles filters"]
    fn renders_h264_aac_video_with_burned_and_sidecar_subtitles() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let database = directory.path().join("zhihua.sqlite3");
        let projects = ProjectStorage::initialize(&database, directory.path().join("projects"))
            .expect("projects");
        let exporter = FfmpegExporter::new(projects);
        let workspace = directory.path().join("work");
        let output = directory.path().join("output");
        fs::create_dir_all(&workspace).expect("workspace");
        fs::create_dir_all(&output).expect("output");
        let video = directory.path().join("input.mp4");
        let audio = directory.path().join("input.wav");
        let music = directory.path().join("music.wav");
        run_ffmpeg(&[
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=320x180:d=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:duration=1",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            "-shortest",
            "-pix_fmt",
            "yuv420p",
            path_text(&video).expect("video path"),
        ])
        .expect("fixture video");
        run_ffmpeg(&[
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-c:a",
            "pcm_s16le",
            path_text(&audio).expect("audio path"),
        ])
        .expect("fixture audio");
        run_ffmpeg(&[
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=220:duration=1",
            "-c:a",
            "pcm_s16le",
            path_text(&music).expect("music path"),
        ])
        .expect("fixture music");
        let result = exporter
            .render(
                &workspace,
                &output,
                "导出测试",
                &[ExportScene {
                    narration: "这是字幕测试。".to_owned(),
                    duration_ms: 1_000,
                    video_path: video,
                    narration_path: Some(audio),
                    include_environment: true,
                    uses_enhanced_source: false,
                }],
                Some(&music),
                &ExportProjectInput {
                    project_id: "unused".to_owned(),
                    output_directory: output.clone(),
                    frame_rate: 24,
                    subtitle_mode: SubtitleMode::BurnAndSrt,
                    aspect_ratio: FrameAspectRatio::Landscape,
                    rendition: OutputRendition::Enhanced1080p,
                    narration_volume: 80,
                    music_asset_id: Some("unused-in-render".into()),
                    music_volume: 60,
                    music_fade: true,
                    environment_audio_policy: EnvironmentAudioPolicy::Smart,
                    environment_volume: 28,
                },
            )
            .expect("render export");
        assert!(result.output_path.is_file());
        assert!(result.subtitle_path.expect("subtitle path").is_file());
        let probe = command_output(
            "ffprobe",
            &[
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name",
                "-of",
                "csv=p=0",
                path_text(&result.output_path).expect("output path"),
            ],
        )
        .expect("ffprobe");
        let codecs = String::from_utf8_lossy(&probe.stdout);
        assert!(codecs.contains("h264"));
        assert!(codecs.contains("aac"));
    }
}
