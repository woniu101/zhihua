use crate::storage::ProjectStorage;
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, error::Error, fmt, path::PathBuf, time::Duration};

#[derive(Debug)]
pub struct StoryboardError(String);

impl StoryboardError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for StoryboardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for StoryboardError {}

impl From<rusqlite::Error> for StoryboardError {
    fn from(error: rusqlite::Error) -> Self {
        Self::new(format!("分镜数据库操作失败：{error}"))
    }
}

impl From<serde_json::Error> for StoryboardError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(format!("分镜数据序列化失败：{error}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GenerationMode {
    T2v,
    I2v,
    Flf2v,
    R2v,
    Continue,
}

impl GenerationMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::T2v => "t2v",
            Self::I2v => "i2v",
            Self::Flf2v => "flf2v",
            Self::R2v => "r2v",
            Self::Continue => "continue",
        }
    }

    fn from_database(value: &str) -> Result<Self, StoryboardError> {
        match value {
            "t2v" => Ok(Self::T2v),
            "i2v" => Ok(Self::I2v),
            "flf2v" => Ok(Self::Flf2v),
            "r2v" => Ok(Self::R2v),
            "continue" => Ok(Self::Continue),
            _ => Err(StoryboardError::new(format!("未知的分镜生成方式：{value}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SceneStatus {
    Draft,
    Ready,
    Generating,
    Generated,
    Approved,
    Failed,
}

impl SceneStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Ready => "ready",
            Self::Generating => "generating",
            Self::Generated => "generated",
            Self::Approved => "approved",
            Self::Failed => "failed",
        }
    }

    fn from_database(value: &str) -> Result<Self, StoryboardError> {
        match value {
            "draft" => Ok(Self::Draft),
            "ready" => Ok(Self::Ready),
            "generating" => Ok(Self::Generating),
            "generated" => Ok(Self::Generated),
            "approved" => Ok(Self::Approved),
            "failed" => Ok(Self::Failed),
            _ => Err(StoryboardError::new(format!("未知的分镜状态：{value}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CandidateQuality {
    Fast,
    High,
}

impl CandidateQuality {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::High => "high",
        }
    }

    fn from_database(value: &str) -> Result<Self, StoryboardError> {
        match value {
            "fast" => Ok(Self::Fast),
            "high" => Ok(Self::High),
            _ => Err(StoryboardError::new(format!("未知的候选质量：{value}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NarrationMode {
    Tts,
    Imported,
    None,
}

impl NarrationMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tts => "tts",
            Self::Imported => "imported",
            Self::None => "none",
        }
    }

    fn from_database(value: &str) -> Result<Self, StoryboardError> {
        match value {
            "tts" => Ok(Self::Tts),
            "imported" => Ok(Self::Imported),
            "none" => Ok(Self::None),
            _ => Err(StoryboardError::new(format!("未知的旁白方式：{value}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReference {
    pub source_id: String,
    pub page: Option<u32>,
    pub paragraph: Option<u32>,
    pub quote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneDraft {
    pub id: String,
    pub project_id: String,
    pub order: u32,
    pub title: String,
    pub purpose: String,
    pub source_refs: Vec<SourceReference>,
    pub narration: String,
    pub narration_mode: NarrationMode,
    pub ambient_sound: String,
    pub on_screen_text: Vec<String>,
    pub visual_plan: String,
    pub generation_mode: GenerationMode,
    pub target_duration_ms: u32,
    pub asset_ids: Vec<String>,
    pub selected_version_id: Option<String>,
    pub last_job_id: Option<String>,
    pub last_upscale_job_id: Option<String>,
    pub pending_request_id: Option<String>,
    pub generation_stage: Option<String>,
    pub status: SceneStatus,
    pub quality: CandidateQuality,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderScenesInput {
    pub project_id: String,
    pub ordered_scene_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StoryboardStorage {
    project_storage: ProjectStorage,
    database_path: PathBuf,
}

impl StoryboardStorage {
    pub fn initialize(project_storage: ProjectStorage) -> Result<Self, StoryboardError> {
        let database_path = project_storage.info().database_path;
        let storage = Self {
            project_storage,
            database_path,
        };
        let connection = storage.connection()?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS storyboard_scenes (
                id                    TEXT PRIMARY KEY NOT NULL,
                project_id            TEXT NOT NULL,
                order_index           INTEGER NOT NULL,
                title                 TEXT NOT NULL,
                purpose               TEXT NOT NULL DEFAULT '',
                source_refs_json      TEXT NOT NULL DEFAULT '[]',
                narration             TEXT NOT NULL DEFAULT '',
                narration_mode        TEXT NOT NULL DEFAULT 'tts',
                ambient_sound         TEXT NOT NULL DEFAULT '',
                on_screen_text_json   TEXT NOT NULL DEFAULT '[]',
                visual_plan           TEXT NOT NULL DEFAULT '',
                generation_mode       TEXT NOT NULL,
                target_duration_ms    INTEGER NOT NULL,
                asset_ids_json        TEXT NOT NULL DEFAULT '[]',
                selected_version_id   TEXT,
                last_job_id           TEXT,
                last_upscale_job_id   TEXT,
                pending_request_id    TEXT,
                generation_stage      TEXT,
                status                TEXT NOT NULL,
                quality               TEXT NOT NULL,
                updated_at            TEXT NOT NULL,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_storyboard_scenes_project_order
                ON storyboard_scenes(project_id, order_index ASC, id ASC);
            ",
        )?;
        let has_upscale_job = {
            let mut statement = connection.prepare("PRAGMA table_info(storyboard_scenes)")?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            columns.iter().any(|column| column == "last_upscale_job_id")
        };
        if !has_upscale_job {
            connection.execute(
                "ALTER TABLE storyboard_scenes ADD COLUMN last_upscale_job_id TEXT",
                [],
            )?;
        }
        let columns = {
            let mut statement = connection.prepare("PRAGMA table_info(storyboard_scenes)")?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            columns
        };
        if !columns.iter().any(|column| column == "narration_mode") {
            connection.execute(
                "ALTER TABLE storyboard_scenes ADD COLUMN narration_mode TEXT NOT NULL DEFAULT 'tts'",
                [],
            )?;
        }
        if !columns.iter().any(|column| column == "ambient_sound") {
            connection.execute(
                "ALTER TABLE storyboard_scenes ADD COLUMN ambient_sound TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        Ok(storage)
    }

    fn connection(&self) -> Result<Connection, StoryboardError> {
        let connection = Connection::open(&self.database_path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Ok(connection)
    }

    pub fn list(&self, project_id: &str) -> Result<Vec<SceneDraft>, StoryboardError> {
        self.ensure_project(project_id)?;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, project_id, order_index, title, purpose, source_refs_json,
                    narration, narration_mode, ambient_sound, on_screen_text_json, visual_plan, generation_mode,
                    target_duration_ms, asset_ids_json, selected_version_id, last_job_id,
                    last_upscale_job_id, pending_request_id, generation_stage, status, quality, updated_at
             FROM storyboard_scenes WHERE project_id = ?1
             ORDER BY order_index ASC, id ASC",
        )?;
        let rows = statement.query_map([project_id], scene_from_row)?;
        rows.map(|row| row.map_err(StoryboardError::from)).collect()
    }

    pub fn upsert(&self, mut scene: SceneDraft) -> Result<SceneDraft, StoryboardError> {
        self.ensure_project(&scene.project_id)?;
        validate_scene(&scene)?;
        scene.updated_at = now_iso();
        let source_refs = serde_json::to_string(&scene.source_refs)?;
        let on_screen_text = serde_json::to_string(&scene.on_screen_text)?;
        let asset_ids = serde_json::to_string(&scene.asset_ids)?;
        let connection = self.connection()?;
        let changed = connection.execute(
            "INSERT INTO storyboard_scenes (
                id, project_id, order_index, title, purpose, source_refs_json, narration,
                narration_mode, ambient_sound, on_screen_text_json, visual_plan, generation_mode, target_duration_ms,
                asset_ids_json, selected_version_id, last_job_id, last_upscale_job_id,
                pending_request_id, generation_stage, status, quality, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22
             )
             ON CONFLICT(id) DO UPDATE SET
                order_index = excluded.order_index,
                title = excluded.title,
                purpose = excluded.purpose,
                source_refs_json = excluded.source_refs_json,
                narration = excluded.narration,
                narration_mode = excluded.narration_mode,
                ambient_sound = excluded.ambient_sound,
                on_screen_text_json = excluded.on_screen_text_json,
                visual_plan = excluded.visual_plan,
                generation_mode = excluded.generation_mode,
                target_duration_ms = excluded.target_duration_ms,
                asset_ids_json = excluded.asset_ids_json,
                selected_version_id = excluded.selected_version_id,
                last_job_id = excluded.last_job_id,
                last_upscale_job_id = excluded.last_upscale_job_id,
                pending_request_id = excluded.pending_request_id,
                generation_stage = excluded.generation_stage,
                status = excluded.status,
                quality = excluded.quality,
                updated_at = excluded.updated_at
             WHERE storyboard_scenes.project_id = excluded.project_id",
            params![
                scene.id,
                scene.project_id,
                scene.order,
                scene.title,
                scene.purpose,
                source_refs,
                scene.narration,
                scene.narration_mode.as_str(),
                scene.ambient_sound,
                on_screen_text,
                scene.visual_plan,
                scene.generation_mode.as_str(),
                scene.target_duration_ms,
                asset_ids,
                scene.selected_version_id,
                scene.last_job_id,
                scene.last_upscale_job_id,
                scene.pending_request_id,
                scene.generation_stage,
                scene.status.as_str(),
                scene.quality.as_str(),
                scene.updated_at,
            ],
        )?;
        if changed != 1 {
            return Err(StoryboardError::new(
                "分镜 ID 已属于其他项目，不能移动到当前项目",
            ));
        }
        Ok(scene)
    }

    pub fn delete(&self, project_id: &str, id: &str) -> Result<(), StoryboardError> {
        self.ensure_project(project_id)?;
        validate_identifier(id, "分镜 ID")?;
        let connection = self.connection()?;
        let changed = connection.execute(
            "DELETE FROM storyboard_scenes WHERE id = ?1 AND project_id = ?2",
            params![id, project_id],
        )?;
        if changed != 1 {
            return Err(StoryboardError::new("分镜不存在或不属于当前项目"));
        }
        normalize_order(&connection, project_id)?;
        Ok(())
    }

    pub fn reorder(&self, input: ReorderScenesInput) -> Result<Vec<SceneDraft>, StoryboardError> {
        self.ensure_project(&input.project_id)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        validate_complete_order(&transaction, &input.project_id, &input.ordered_scene_ids)?;
        for (order, id) in input.ordered_scene_ids.iter().enumerate() {
            transaction.execute(
                "UPDATE storyboard_scenes SET order_index = ?1, updated_at = ?2
                 WHERE id = ?3 AND project_id = ?4",
                params![order as u32, now_iso(), id, input.project_id],
            )?;
        }
        transaction.commit()?;
        self.list(&input.project_id)
    }

    fn ensure_project(&self, project_id: &str) -> Result<(), StoryboardError> {
        self.project_storage
            .get_project(project_id)
            .map(|_| ())
            .map_err(|error| StoryboardError::new(error.to_string()))
    }
}

fn validate_scene(scene: &SceneDraft) -> Result<(), StoryboardError> {
    validate_identifier(&scene.id, "分镜 ID")?;
    if scene.title.trim().is_empty() {
        return Err(StoryboardError::new("分镜标题不能为空"));
    }
    if scene.title.chars().count() > 200 {
        return Err(StoryboardError::new("分镜标题不能超过 200 个字符"));
    }
    if !matches!(scene.target_duration_ms, 5_000 | 10_000 | 15_000) {
        return Err(StoryboardError::new("分镜时长只能是 5、10 或 15 秒"));
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str) -> Result<(), StoryboardError> {
    let length = value.chars().count();
    if value.trim().is_empty() || length > 128 {
        return Err(StoryboardError::new(format!(
            "{label}不能为空且不能超过 128 个字符"
        )));
    }
    Ok(())
}

fn validate_complete_order(
    transaction: &Transaction<'_>,
    project_id: &str,
    ordered_ids: &[String],
) -> Result<(), StoryboardError> {
    let unique: HashSet<&str> = ordered_ids.iter().map(String::as_str).collect();
    if unique.len() != ordered_ids.len() {
        return Err(StoryboardError::new("分镜排序列表中存在重复 ID"));
    }

    let mut statement = transaction
        .prepare("SELECT id FROM storyboard_scenes WHERE project_id = ?1 ORDER BY id ASC")?;
    let actual = statement
        .query_map([project_id], |row| row.get::<_, String>(0))?
        .collect::<Result<HashSet<_>, _>>()?;
    let requested: HashSet<String> = ordered_ids.iter().cloned().collect();
    if requested != actual {
        return Err(StoryboardError::new("排序必须且只能包含当前项目的全部分镜"));
    }
    Ok(())
}

fn normalize_order(connection: &Connection, project_id: &str) -> Result<(), StoryboardError> {
    let mut statement = connection.prepare(
        "SELECT id FROM storyboard_scenes WHERE project_id = ?1
         ORDER BY order_index ASC, id ASC",
    )?;
    let ids = statement
        .query_map([project_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (order, id) in ids.iter().enumerate() {
        connection.execute(
            "UPDATE storyboard_scenes SET order_index = ?1 WHERE id = ?2 AND project_id = ?3",
            params![order as u32, id, project_id],
        )?;
    }
    Ok(())
}

fn scene_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SceneDraft> {
    let order: i64 = row.get(2)?;
    let target_duration: i64 = row.get(12)?;
    let source_refs_json: String = row.get(5)?;
    let narration_mode: String = row.get(7)?;
    let on_screen_text_json: String = row.get(9)?;
    let asset_ids_json: String = row.get(13)?;
    let generation_mode: String = row.get(11)?;
    let status: String = row.get(19)?;
    let quality: String = row.get(20)?;

    Ok(SceneDraft {
        id: row.get(0)?,
        project_id: row.get(1)?,
        order: u32::try_from(order).map_err(|error| conversion_error(2, error))?,
        title: row.get(3)?,
        purpose: row.get(4)?,
        source_refs: serde_json::from_str(&source_refs_json)
            .map_err(|error| conversion_error(5, error))?,
        narration: row.get(6)?,
        narration_mode: NarrationMode::from_database(&narration_mode)
            .map_err(|error| conversion_error(7, error))?,
        ambient_sound: row.get(8)?,
        on_screen_text: serde_json::from_str(&on_screen_text_json)
            .map_err(|error| conversion_error(9, error))?,
        visual_plan: row.get(10)?,
        generation_mode: GenerationMode::from_database(&generation_mode)
            .map_err(|error| conversion_error(11, error))?,
        target_duration_ms: u32::try_from(target_duration)
            .map_err(|error| conversion_error(12, error))?,
        asset_ids: serde_json::from_str(&asset_ids_json)
            .map_err(|error| conversion_error(13, error))?,
        selected_version_id: row.get(14)?,
        last_job_id: row.get(15)?,
        last_upscale_job_id: row.get(16)?,
        pending_request_id: row.get(17)?,
        generation_stage: row.get(18)?,
        status: SceneStatus::from_database(&status).map_err(|error| conversion_error(19, error))?,
        quality: CandidateQuality::from_database(&quality)
            .map_err(|error| conversion_error(20, error))?,
        updated_at: row.get(21)?,
    })
}

fn conversion_error(column: usize, error: impl Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Text, Box::new(error))
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateProjectInput, Project};

    fn storage() -> (tempfile::TempDir, ProjectStorage, StoryboardStorage) {
        let directory = tempfile::tempdir().expect("temporary directory");
        let project_storage = ProjectStorage::initialize(
            directory.path().join("zhihua.sqlite3"),
            directory.path().join("projects"),
        )
        .expect("initialize projects");
        let storyboard =
            StoryboardStorage::initialize(project_storage.clone()).expect("initialize storyboard");
        (directory, project_storage, storyboard)
    }

    fn project(storage: &ProjectStorage, title: &str) -> Project {
        storage
            .create_project(CreateProjectInput {
                title: title.to_owned(),
                audience: None,
                target_duration_sec: Some(60),
            })
            .expect("create project")
    }

    fn scene(project_id: &str, id: &str, order: u32) -> SceneDraft {
        SceneDraft {
            id: id.to_owned(),
            project_id: project_id.to_owned(),
            order,
            title: format!("镜头 {id}"),
            purpose: "解释原理".to_owned(),
            source_refs: vec![SourceReference {
                source_id: "source-1".to_owned(),
                page: Some(2),
                paragraph: None,
                quote: Some("引用".to_owned()),
            }],
            narration: "旁白".to_owned(),
            narration_mode: NarrationMode::Tts,
            ambient_sound: "雨声和远处雷声".to_owned(),
            on_screen_text: vec!["字幕".to_owned()],
            visual_plan: "画面描述".to_owned(),
            generation_mode: GenerationMode::R2v,
            target_duration_ms: 10_000,
            asset_ids: vec!["asset-1".to_owned()],
            selected_version_id: Some("version-1".to_owned()),
            last_job_id: Some("job-1".to_owned()),
            last_upscale_job_id: Some("upscale-1".to_owned()),
            pending_request_id: Some("request-1".to_owned()),
            generation_stage: Some("生成中".to_owned()),
            status: SceneStatus::Generating,
            quality: CandidateQuality::High,
            updated_at: String::new(),
        }
    }

    #[test]
    fn persists_complete_scene_and_lists_by_project_order() {
        let (_directory, projects, storyboard) = storage();
        let project = project(&projects, "雷电");
        let second = storyboard
            .upsert(scene(&project.id, "scene-2", 1))
            .expect("insert second scene");
        let first = storyboard
            .upsert(scene(&project.id, "scene-1", 0))
            .expect("insert first scene");

        let listed = storyboard.list(&project.id).expect("list scenes");
        assert_eq!(listed, vec![first, second]);
        assert_eq!(listed[0].generation_mode, GenerationMode::R2v);
        assert_eq!(listed[0].last_job_id.as_deref(), Some("job-1"));
        assert_eq!(listed[0].last_upscale_job_id.as_deref(), Some("upscale-1"));
        assert_eq!(listed[0].pending_request_id.as_deref(), Some("request-1"));
    }

    #[test]
    fn rejects_cross_project_scene_reassignment() {
        let (_directory, projects, storyboard) = storage();
        let first_project = project(&projects, "项目一");
        let second_project = project(&projects, "项目二");
        storyboard
            .upsert(scene(&first_project.id, "shared-scene", 0))
            .expect("insert scene");

        let error = storyboard
            .upsert(scene(&second_project.id, "shared-scene", 0))
            .expect_err("moving scene across projects must fail");
        assert!(error.to_string().contains("其他项目"));
        assert!(storyboard
            .list(&second_project.id)
            .expect("list second project")
            .is_empty());
    }

    #[test]
    fn reorders_only_with_a_complete_unique_project_scene_list() {
        let (_directory, projects, storyboard) = storage();
        let project = project(&projects, "雷电");
        for (order, id) in ["a", "b", "c"].into_iter().enumerate() {
            storyboard
                .upsert(scene(&project.id, id, order as u32))
                .expect("insert scene");
        }

        let error = storyboard
            .reorder(ReorderScenesInput {
                project_id: project.id.clone(),
                ordered_scene_ids: vec!["a".to_owned(), "a".to_owned(), "c".to_owned()],
            })
            .expect_err("duplicate ids must fail");
        assert!(error.to_string().contains("重复"));

        let reordered = storyboard
            .reorder(ReorderScenesInput {
                project_id: project.id,
                ordered_scene_ids: vec!["c".to_owned(), "a".to_owned(), "b".to_owned()],
            })
            .expect("reorder scenes");
        assert_eq!(
            reordered
                .iter()
                .map(|scene| scene.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c", "a", "b"]
        );
        assert_eq!(
            reordered
                .iter()
                .map(|scene| scene.order)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn delete_cascades_and_project_copy_starts_without_database_scenes() {
        let (_directory, projects, storyboard) = storage();
        let original = project(&projects, "雷电");
        storyboard
            .upsert(scene(&original.id, "scene-1", 0))
            .expect("insert scene");

        let copy = projects
            .duplicate_project(&original.id, None)
            .expect("duplicate project");
        assert!(storyboard
            .list(&copy.id)
            .expect("list copied scenes")
            .is_empty());

        projects
            .delete_project(&original.id)
            .expect("delete original project");
        let count: i64 = storyboard
            .connection()
            .expect("connect")
            .query_row(
                "SELECT COUNT(*) FROM storyboard_scenes WHERE project_id = ?1",
                [&original.id],
                |row| row.get(0),
            )
            .expect("count scenes");
        assert_eq!(count, 0);
        assert!(storyboard
            .list(&copy.id)
            .expect("copy still exists")
            .is_empty());
    }

    #[test]
    fn validates_allowed_mvp_duration_and_normalizes_order_after_delete() {
        let (_directory, projects, storyboard) = storage();
        let project = project(&projects, "雷电");
        let mut invalid = scene(&project.id, "invalid", 0);
        invalid.target_duration_ms = 8_000;
        assert!(storyboard
            .upsert(invalid)
            .expect_err("invalid duration")
            .to_string()
            .contains("5、10 或 15"));

        storyboard
            .upsert(scene(&project.id, "a", 0))
            .expect("insert a");
        storyboard
            .upsert(scene(&project.id, "b", 1))
            .expect("insert b");
        storyboard
            .upsert(scene(&project.id, "c", 2))
            .expect("insert c");
        storyboard.delete(&project.id, "b").expect("delete b");
        let listed = storyboard.list(&project.id).expect("list scenes");
        assert_eq!(
            listed.iter().map(|scene| scene.order).collect::<Vec<_>>(),
            vec![0, 1]
        );
    }
}
