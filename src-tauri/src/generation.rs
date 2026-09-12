use crate::{frame_profile::FrameAspectRatio, storage::ProjectStorage};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::{fmt, path::PathBuf, time::Duration};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationError {
    pub code: String,
    pub message: String,
}

impl GenerationError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for GenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

type GenerationResult<T> = Result<T, GenerationError>;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateVersion {
    pub id: String,
    pub project_id: String,
    pub scene_id: String,
    pub job_id: String,
    pub workflow_id: String,
    pub prompt_id: Option<String>,
    pub prompt_compiler_version: Option<String>,
    pub h3_audio_policy: Option<String>,
    pub artifact_id: String,
    pub filename: String,
    pub media_type: String,
    pub local_path: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
    pub selected: bool,
    pub created_at: String,
    pub aspect_ratio: String,
    pub work_width: u32,
    pub work_height: u32,
    pub visible_width: u32,
    pub visible_height: u32,
    pub crop_x: u32,
    pub crop_y: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhancedVersion {
    pub id: String,
    pub project_id: String,
    pub scene_id: String,
    pub source_candidate_id: String,
    pub job_id: String,
    pub workflow_id: String,
    pub prompt_id: Option<String>,
    pub artifact_id: String,
    pub filename: String,
    pub media_type: String,
    pub local_path: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct RecordCandidateInput {
    pub project_id: String,
    pub scene_id: String,
    pub job_id: String,
    pub workflow_id: String,
    pub prompt_id: Option<String>,
    pub prompt_compiler_version: Option<String>,
    pub h3_audio_policy: Option<String>,
    pub artifact_id: String,
    pub filename: String,
    pub media_type: String,
    pub local_path: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
    pub aspect_ratio: String,
    pub work_width: u32,
    pub work_height: u32,
    pub visible_width: u32,
    pub visible_height: u32,
    pub crop_x: u32,
    pub crop_y: u32,
}

#[derive(Clone, Debug)]
pub struct RecordEnhancedInput {
    pub project_id: String,
    pub scene_id: String,
    pub source_candidate_id: String,
    pub job_id: String,
    pub workflow_id: String,
    pub prompt_id: Option<String>,
    pub artifact_id: String,
    pub filename: String,
    pub media_type: String,
    pub local_path: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone)]
pub struct GenerationStorage {
    database_path: PathBuf,
    project_storage: ProjectStorage,
}

impl GenerationStorage {
    pub fn initialize(project_storage: ProjectStorage) -> GenerationResult<Self> {
        let storage = Self {
            database_path: project_storage.info().database_path,
            project_storage,
        };
        let connection = storage.connection()?;
        connection
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS candidate_versions (
                id          TEXT PRIMARY KEY NOT NULL,
                project_id  TEXT NOT NULL,
                scene_id    TEXT NOT NULL,
                job_id      TEXT NOT NULL,
                workflow_id TEXT NOT NULL,
                prompt_id   TEXT,
                prompt_compiler_version TEXT,
                h3_audio_policy TEXT,
                artifact_id TEXT NOT NULL,
                filename    TEXT NOT NULL,
                media_type  TEXT NOT NULL,
                local_path  TEXT NOT NULL UNIQUE,
                size_bytes  INTEGER NOT NULL,
                sha256      TEXT NOT NULL,
                selected    INTEGER NOT NULL DEFAULT 0,
                created_at  TEXT NOT NULL,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE,
                UNIQUE(job_id, artifact_id)
            );
            CREATE INDEX IF NOT EXISTS idx_candidate_versions_scene
                ON candidate_versions(project_id, scene_id, created_at DESC);
            CREATE TABLE IF NOT EXISTS candidate_frame_profiles (
                candidate_id   TEXT PRIMARY KEY NOT NULL,
                aspect_ratio   TEXT NOT NULL,
                work_width     INTEGER NOT NULL,
                work_height    INTEGER NOT NULL,
                visible_width  INTEGER NOT NULL,
                visible_height INTEGER NOT NULL,
                crop_x         INTEGER NOT NULL,
                crop_y         INTEGER NOT NULL,
                FOREIGN KEY(candidate_id) REFERENCES candidate_versions(id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS enhanced_versions (
                id                  TEXT PRIMARY KEY NOT NULL,
                project_id          TEXT NOT NULL,
                scene_id            TEXT NOT NULL,
                source_candidate_id TEXT NOT NULL,
                job_id              TEXT NOT NULL,
                workflow_id         TEXT NOT NULL,
                prompt_id           TEXT,
                artifact_id         TEXT NOT NULL,
                filename            TEXT NOT NULL,
                media_type          TEXT NOT NULL,
                local_path          TEXT NOT NULL UNIQUE,
                size_bytes          INTEGER NOT NULL,
                sha256              TEXT NOT NULL,
                created_at          TEXT NOT NULL,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE,
                FOREIGN KEY(source_candidate_id) REFERENCES candidate_versions(id),
                UNIQUE(job_id, artifact_id)
            );
            CREATE INDEX IF NOT EXISTS idx_enhanced_versions_scene
                ON enhanced_versions(project_id, scene_id, created_at DESC);
            ",
            )
            .map_err(database_error)?;
        let candidate_columns = connection
            .prepare("PRAGMA table_info(candidate_versions)")
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| row.get::<_, String>(1))?
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(database_error)?;
        if !candidate_columns
            .iter()
            .any(|column| column == "prompt_compiler_version")
        {
            connection
                .execute(
                    "ALTER TABLE candidate_versions ADD COLUMN prompt_compiler_version TEXT",
                    [],
                )
                .map_err(database_error)?;
        }
        if !candidate_columns
            .iter()
            .any(|column| column == "h3_audio_policy")
        {
            connection
                .execute(
                    "ALTER TABLE candidate_versions ADD COLUMN h3_audio_policy TEXT",
                    [],
                )
                .map_err(database_error)?;
        }
        Ok(storage)
    }

    fn connection(&self) -> GenerationResult<Connection> {
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

    pub fn list(
        &self,
        project_id: &str,
        scene_id: &str,
    ) -> GenerationResult<Vec<CandidateVersion>> {
        self.project_storage
            .get_project(project_id)
            .map_err(|error| GenerationError::new("project_unavailable", error.to_string()))?;
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT c.id, c.project_id, c.scene_id, c.job_id, c.workflow_id, c.prompt_id,
                        c.prompt_compiler_version, c.h3_audio_policy,
                        c.artifact_id, c.filename, c.media_type, c.local_path, c.size_bytes,
                        c.sha256, c.selected, c.created_at, f.aspect_ratio, f.work_width,
                        f.work_height, f.visible_width, f.visible_height, f.crop_x, f.crop_y
                 FROM candidate_versions c
                 JOIN candidate_frame_profiles f ON f.candidate_id = c.id
                 WHERE c.project_id = ?1 AND c.scene_id = ?2
                 ORDER BY c.created_at ASC, c.id ASC",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(params![project_id, scene_id], candidate_from_row)
            .map_err(database_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(database_error)
    }

    pub fn list_enhanced(
        &self,
        project_id: &str,
        scene_id: &str,
    ) -> GenerationResult<Vec<EnhancedVersion>> {
        self.project_storage
            .get_project(project_id)
            .map_err(|error| GenerationError::new("project_unavailable", error.to_string()))?;
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, project_id, scene_id, source_candidate_id, job_id,
                        workflow_id, prompt_id, artifact_id, filename, media_type,
                        local_path, size_bytes, sha256, created_at
                 FROM enhanced_versions
                 WHERE project_id = ?1 AND scene_id = ?2
                 ORDER BY created_at ASC, id ASC",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(params![project_id, scene_id], enhanced_from_row)
            .map_err(database_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(database_error)
    }

    pub fn record(&self, input: RecordCandidateInput) -> GenerationResult<CandidateVersion> {
        self.project_storage
            .get_project(&input.project_id)
            .map_err(|error| GenerationError::new("project_unavailable", error.to_string()))?;
        if !input.local_path.is_absolute() || !input.local_path.is_file() {
            return Err(GenerationError::new(
                "candidate_file_unavailable",
                "候选视频尚未完整保存到项目目录",
            ));
        }
        let profile = FrameAspectRatio::from_label(&input.aspect_ratio)
            .map(FrameAspectRatio::profile)
            .ok_or_else(|| GenerationError::new("invalid_frame_profile", "候选视频画幅无效"))?;
        if input.aspect_ratio != profile.aspect_ratio.label()
            || input.work_width != profile.work.width
            || input.work_height != profile.work.height
            || input.visible_width != profile.visible.width
            || input.visible_height != profile.visible.height
            || input.crop_x != profile.crop_x
            || input.crop_y != profile.crop_y
        {
            return Err(GenerationError::new(
                "invalid_frame_profile",
                "候选视频尺寸与知画画幅契约不一致",
            ));
        }
        let id = Uuid::new_v4().to_string();
        let created_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(database_error)?;
        transaction
            .execute(
                "INSERT INTO candidate_versions (
                    id, project_id, scene_id, job_id, workflow_id, prompt_id,
                    prompt_compiler_version, h3_audio_policy, artifact_id, filename,
                    media_type, local_path, size_bytes, sha256, selected, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 0, ?15)
                 ON CONFLICT(job_id, artifact_id) DO UPDATE SET
                    local_path = excluded.local_path,
                    size_bytes = excluded.size_bytes,
                    sha256 = excluded.sha256,
                    prompt_compiler_version = excluded.prompt_compiler_version,
                    h3_audio_policy = excluded.h3_audio_policy",
                params![
                    id,
                    input.project_id,
                    input.scene_id,
                    input.job_id,
                    input.workflow_id,
                    input.prompt_id,
                    input.prompt_compiler_version,
                    input.h3_audio_policy,
                    input.artifact_id,
                    input.filename,
                    input.media_type,
                    input.local_path.to_string_lossy(),
                    input.size_bytes,
                    input.sha256,
                    created_at,
                ],
            )
            .map_err(database_error)?;
        let candidate_id: String = transaction
            .query_row(
                "SELECT id FROM candidate_versions WHERE job_id = ?1 AND artifact_id = ?2",
                params![input.job_id, input.artifact_id],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        transaction
            .execute(
                "INSERT INTO candidate_frame_profiles (
                    candidate_id, aspect_ratio, work_width, work_height,
                    visible_width, visible_height, crop_x, crop_y
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(candidate_id) DO UPDATE SET
                    aspect_ratio=excluded.aspect_ratio,
                    work_width=excluded.work_width,
                    work_height=excluded.work_height,
                    visible_width=excluded.visible_width,
                    visible_height=excluded.visible_height,
                    crop_x=excluded.crop_x,
                    crop_y=excluded.crop_y",
                params![
                    candidate_id,
                    input.aspect_ratio,
                    input.work_width,
                    input.work_height,
                    input.visible_width,
                    input.visible_height,
                    input.crop_x,
                    input.crop_y,
                ],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)?;
        self.find_by_job_artifact(&input.job_id, &input.artifact_id)
    }

    pub fn select(
        &self,
        project_id: &str,
        scene_id: &str,
        version_id: &str,
    ) -> GenerationResult<CandidateVersion> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(database_error)?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM candidate_versions
                    WHERE id = ?1 AND project_id = ?2 AND scene_id = ?3
                 )",
                params![version_id, project_id, scene_id],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        if !exists {
            return Err(GenerationError::new(
                "candidate_not_found",
                "找不到要设为正式版本的候选视频",
            ));
        }
        transaction
            .execute(
                "UPDATE candidate_versions SET selected = CASE WHEN id = ?1 THEN 1 ELSE 0 END
                 WHERE project_id = ?2 AND scene_id = ?3",
                params![version_id, project_id, scene_id],
            )
            .map_err(database_error)?;
        transaction
            .execute(
                "UPDATE storyboard_scenes
                 SET selected_version_id = ?1, status = 'approved', updated_at = ?2
                 WHERE id = ?3 AND project_id = ?4",
                params![version_id, Utc::now().to_rfc3339(), scene_id, project_id],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)?;
        self.find(version_id)
    }

    pub fn find(&self, id: &str) -> GenerationResult<CandidateVersion> {
        self.connection()?
            .query_row(
                "SELECT c.id, c.project_id, c.scene_id, c.job_id, c.workflow_id, c.prompt_id,
                        c.prompt_compiler_version, c.h3_audio_policy,
                        c.artifact_id, c.filename, c.media_type, c.local_path, c.size_bytes,
                        c.sha256, c.selected, c.created_at, f.aspect_ratio, f.work_width,
                        f.work_height, f.visible_width, f.visible_height, f.crop_x, f.crop_y
                 FROM candidate_versions c
                 JOIN candidate_frame_profiles f ON f.candidate_id = c.id
                 WHERE c.id = ?1",
                [id],
                candidate_from_row,
            )
            .map_err(database_error)
    }

    fn find_by_job_artifact(
        &self,
        job_id: &str,
        artifact_id: &str,
    ) -> GenerationResult<CandidateVersion> {
        self.connection()?
            .query_row(
                "SELECT c.id, c.project_id, c.scene_id, c.job_id, c.workflow_id, c.prompt_id,
                        c.prompt_compiler_version, c.h3_audio_policy,
                        c.artifact_id, c.filename, c.media_type, c.local_path, c.size_bytes,
                        c.sha256, c.selected, c.created_at, f.aspect_ratio, f.work_width,
                        f.work_height, f.visible_width, f.visible_height, f.crop_x, f.crop_y
                 FROM candidate_versions c
                 JOIN candidate_frame_profiles f ON f.candidate_id = c.id
                 WHERE c.job_id = ?1 AND c.artifact_id = ?2",
                params![job_id, artifact_id],
                candidate_from_row,
            )
            .map_err(database_error)
    }

    pub fn record_enhanced(&self, input: RecordEnhancedInput) -> GenerationResult<EnhancedVersion> {
        self.project_storage
            .get_project(&input.project_id)
            .map_err(|error| GenerationError::new("project_unavailable", error.to_string()))?;
        let source = self.find(&input.source_candidate_id)?;
        if source.project_id != input.project_id || source.scene_id != input.scene_id {
            return Err(GenerationError::new(
                "source_candidate_mismatch",
                "1080p 增强版的来源候选不属于当前分镜",
            ));
        }
        if !source.selected {
            return Err(GenerationError::new(
                "source_candidate_not_selected",
                "只有正式版本才能制作 1080p 增强版",
            ));
        }
        if !input.local_path.is_absolute() || !input.local_path.is_file() {
            return Err(GenerationError::new(
                "enhanced_file_unavailable",
                "1080p 增强版尚未完整保存到项目目录",
            ));
        }
        let id = Uuid::new_v4().to_string();
        let created_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let connection = self.connection()?;
        connection
            .execute(
                "INSERT INTO enhanced_versions (
                    id, project_id, scene_id, source_candidate_id, job_id,
                    workflow_id, prompt_id, artifact_id, filename, media_type,
                    local_path, size_bytes, sha256, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(job_id, artifact_id) DO UPDATE SET
                    local_path = excluded.local_path,
                    size_bytes = excluded.size_bytes,
                    sha256 = excluded.sha256",
                params![
                    id,
                    input.project_id,
                    input.scene_id,
                    input.source_candidate_id,
                    input.job_id,
                    input.workflow_id,
                    input.prompt_id,
                    input.artifact_id,
                    input.filename,
                    input.media_type,
                    input.local_path.to_string_lossy(),
                    input.size_bytes,
                    input.sha256,
                    created_at,
                ],
            )
            .map_err(database_error)?;
        self.find_enhanced_by_job_artifact(&input.job_id, &input.artifact_id)
    }

    fn find_enhanced_by_job_artifact(
        &self,
        job_id: &str,
        artifact_id: &str,
    ) -> GenerationResult<EnhancedVersion> {
        self.connection()?
            .query_row(
                "SELECT id, project_id, scene_id, source_candidate_id, job_id,
                        workflow_id, prompt_id, artifact_id, filename, media_type,
                        local_path, size_bytes, sha256, created_at
                 FROM enhanced_versions WHERE job_id = ?1 AND artifact_id = ?2",
                params![job_id, artifact_id],
                enhanced_from_row,
            )
            .map_err(database_error)
    }
}

fn candidate_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CandidateVersion> {
    Ok(CandidateVersion {
        id: row.get(0)?,
        project_id: row.get(1)?,
        scene_id: row.get(2)?,
        job_id: row.get(3)?,
        workflow_id: row.get(4)?,
        prompt_id: row.get(5)?,
        prompt_compiler_version: row.get(6)?,
        h3_audio_policy: row.get(7)?,
        artifact_id: row.get(8)?,
        filename: row.get(9)?,
        media_type: row.get(10)?,
        local_path: PathBuf::from(row.get::<_, String>(11)?),
        size_bytes: row.get(12)?,
        sha256: row.get(13)?,
        selected: row.get(14)?,
        created_at: row.get(15)?,
        aspect_ratio: row.get(16)?,
        work_width: row.get(17)?,
        work_height: row.get(18)?,
        visible_width: row.get(19)?,
        visible_height: row.get(20)?,
        crop_x: row.get(21)?,
        crop_y: row.get(22)?,
    })
}

fn enhanced_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EnhancedVersion> {
    Ok(EnhancedVersion {
        id: row.get(0)?,
        project_id: row.get(1)?,
        scene_id: row.get(2)?,
        source_candidate_id: row.get(3)?,
        job_id: row.get(4)?,
        workflow_id: row.get(5)?,
        prompt_id: row.get(6)?,
        artifact_id: row.get(7)?,
        filename: row.get(8)?,
        media_type: row.get(9)?,
        local_path: PathBuf::from(row.get::<_, String>(10)?),
        size_bytes: row.get(11)?,
        sha256: row.get(12)?,
        created_at: row.get(13)?,
    })
}

fn database_error(error: rusqlite::Error) -> GenerationError {
    GenerationError::new(
        "generation_database_error",
        format!("候选版本数据库操作失败：{error}"),
    )
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
    fn records_and_selects_candidate_without_removing_older_versions() {
        let directory = tempfile::tempdir().expect("temporary storage");
        let projects = ProjectStorage::initialize(
            directory.path().join("zhihua.sqlite3"),
            directory.path().join("projects"),
        )
        .expect("project storage");
        let project = projects
            .create_project(CreateProjectInput {
                title: "候选版本测试".to_owned(),
                audience: None,
                target_duration_sec: Some(30),
            })
            .expect("project");
        let storyboards = StoryboardStorage::initialize(projects.clone()).expect("storyboards");
        let scene = storyboards
            .upsert(SceneDraft {
                id: Uuid::new_v4().to_string(),
                project_id: project.id.clone(),
                order: 0,
                title: "镜头".to_owned(),
                purpose: String::new(),
                source_refs: vec![],
                narration: String::new(),
                narration_mode: crate::storyboard::NarrationMode::None,
                ambient_sound: "雷声".to_owned(),
                on_screen_text: vec![],
                visual_plan: "闪电".to_owned(),
                generation_mode: GenerationMode::T2v,
                target_duration_ms: 5_000,
                asset_ids: vec![],
                selected_version_id: None,
                last_job_id: None,
                last_upscale_job_id: None,
                pending_request_id: None,
                generation_stage: None,
                status: SceneStatus::Generated,
                quality: CandidateQuality::Fast,
                updated_at: Utc::now().to_rfc3339(),
            })
            .expect("scene");
        let storage = GenerationStorage::initialize(projects).expect("generation storage");
        let path = project.project_dir.join("cache/drafts/clip.mp4");
        std::fs::create_dir_all(path.parent().unwrap()).expect("candidate directory");
        std::fs::write(&path, b"video").expect("candidate");
        let candidate = storage
            .record(RecordCandidateInput {
                project_id: project.id.clone(),
                scene_id: scene.id.clone(),
                job_id: "job-1".to_owned(),
                workflow_id: "h3-t2v-turbo-v1".to_owned(),
                prompt_id: Some("prompt-1".to_owned()),
                prompt_compiler_version: Some("h3-prompt-v1".to_owned()),
                h3_audio_policy: Some("smart".to_owned()),
                artifact_id: "video-0".to_owned(),
                filename: "clip.mp4".to_owned(),
                media_type: "video/mp4".to_owned(),
                local_path: path,
                size_bytes: 5,
                sha256: "a".repeat(64),
                aspect_ratio: "16:9".to_owned(),
                work_width: 1344,
                work_height: 768,
                visible_width: 1344,
                visible_height: 756,
                crop_x: 0,
                crop_y: 6,
            })
            .expect("record");
        storage
            .select(&project.id, &scene.id, &candidate.id)
            .expect("select");
        let versions = storage.list(&project.id, &scene.id).expect("versions");
        assert_eq!(versions.len(), 1);
        assert!(versions[0].selected);
        assert_eq!(
            versions[0].prompt_compiler_version.as_deref(),
            Some("h3-prompt-v1")
        );
        assert_eq!(versions[0].h3_audio_policy.as_deref(), Some("smart"));
        assert_eq!(
            storyboards.list(&project.id).expect("scenes")[0].selected_version_id,
            Some(candidate.id.clone())
        );

        let enhanced_path = project.project_dir.join("cache/enhanced/clip-1080p.mp4");
        std::fs::create_dir_all(enhanced_path.parent().unwrap()).expect("enhanced directory");
        std::fs::write(&enhanced_path, b"upscaled-video").expect("enhanced video");
        let enhanced_version = storage
            .record_enhanced(RecordEnhancedInput {
                project_id: project.id.clone(),
                scene_id: scene.id.clone(),
                source_candidate_id: candidate.id.clone(),
                job_id: "upscale-1".to_owned(),
                workflow_id: "seedvr2-1080p-v1".to_owned(),
                prompt_id: Some("prompt-2".to_owned()),
                artifact_id: "video-1080p".to_owned(),
                filename: "clip-1080p.mp4".to_owned(),
                media_type: "video/mp4".to_owned(),
                local_path: enhanced_path.clone(),
                size_bytes: 14,
                sha256: "b".repeat(64),
            })
            .expect("record enhanced");
        assert_eq!(enhanced_version.source_candidate_id, candidate.id);
        assert_eq!(storage.list(&project.id, &scene.id).unwrap().len(), 1);
        let enhanced = storage.list_enhanced(&project.id, &scene.id).unwrap();
        assert_eq!(enhanced.len(), 1);
        assert_eq!(enhanced[0].local_path, enhanced_path);
    }
}
