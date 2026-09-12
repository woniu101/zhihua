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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AudioIntent {
    #[default]
    Environment,
    Dialogue,
    Silent,
}

impl AudioIntent {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::Dialogue => "dialogue",
            Self::Silent => "silent",
        }
    }

    fn from_database(value: &str) -> Result<Self, StoryboardError> {
        match value {
            "environment" => Ok(Self::Environment),
            "dialogue" => Ok(Self::Dialogue),
            "silent" => Ok(Self::Silent),
            _ => Err(StoryboardError::new(format!("未知的原声音频意图：{value}"))),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VisualIntent {
    pub subject: String,
    pub action: String,
    pub scene: String,
    pub composition: String,
    pub camera: String,
    pub lighting: String,
    pub timeline: String,
    pub negative: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PromptMode {
    #[default]
    Quick,
    Advanced,
}

impl PromptMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Advanced => "advanced",
        }
    }

    fn from_database(value: &str) -> Result<Self, StoryboardError> {
        match value {
            "quick" => Ok(Self::Quick),
            "advanced" => Ok(Self::Advanced),
            _ => Err(StoryboardError::new(format!("未知的提示词模式：{value}"))),
        }
    }
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
    #[serde(default)]
    pub visual_intent: VisualIntent,
    #[serde(default)]
    pub prompt_mode: PromptMode,
    #[serde(default)]
    pub audio_intent: AudioIntent,
    #[serde(default)]
    pub locked: bool,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryboardEditInput {
    pub project_id: String,
    pub expected: Vec<SceneDraft>,
    pub scenes: Vec<SceneDraft>,
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
                locked                INTEGER NOT NULL DEFAULT 0,
                audio_intent          TEXT NOT NULL DEFAULT 'environment',
                visual_intent_json    TEXT NOT NULL DEFAULT '{}',
                prompt_mode           TEXT NOT NULL DEFAULT 'quick',
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
        if !columns.iter().any(|column| column == "locked") {
            connection.execute(
                "ALTER TABLE storyboard_scenes ADD COLUMN locked INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !columns.iter().any(|column| column == "audio_intent") {
            connection.execute(
                "ALTER TABLE storyboard_scenes ADD COLUMN audio_intent TEXT NOT NULL DEFAULT 'environment'",
                [],
            )?;
        }
        if !columns.iter().any(|column| column == "visual_intent_json") {
            connection.execute(
                "ALTER TABLE storyboard_scenes ADD COLUMN visual_intent_json TEXT NOT NULL DEFAULT '{}'",
                [],
            )?;
        }
        if !columns.iter().any(|column| column == "prompt_mode") {
            connection.execute(
                "ALTER TABLE storyboard_scenes ADD COLUMN prompt_mode TEXT NOT NULL DEFAULT 'quick'",
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
        list_with_connection(&connection, project_id)
    }

    pub fn upsert(&self, mut scene: SceneDraft) -> Result<SceneDraft, StoryboardError> {
        self.ensure_project(&scene.project_id)?;
        validate_scene(&scene)?;
        let connection = self.connection()?;
        let existing = list_with_connection(&connection, &scene.project_id)?
            .into_iter()
            .find(|item| item.id == scene.id);
        if let Some(existing) = existing {
            let mut unlock_only = scene.clone();
            unlock_only.locked = existing.locked;
            unlock_only.updated_at = existing.updated_at.clone();
            if existing.locked && unlock_only != existing {
                return Err(StoryboardError::new("分镜已锁定，请先解除锁定再修改内容"));
            }
            if (existing.status == SceneStatus::Generating || existing.pending_request_id.is_some())
                && (!same_authored_content(&existing, &scene) || existing.locked != scene.locked)
            {
                return Err(StoryboardError::new(
                    "分镜正在生成，暂时不能修改内容或锁定状态",
                ));
            }
        }
        scene.updated_at = now_iso();
        write_scene(&connection, scene)
    }

    pub fn delete(&self, project_id: &str, id: &str) -> Result<(), StoryboardError> {
        self.ensure_project(project_id)?;
        validate_identifier(id, "分镜 ID")?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let existing = list_with_connection(&transaction, project_id)?
            .into_iter()
            .find(|scene| scene.id == id)
            .ok_or_else(|| StoryboardError::new("分镜不存在或不属于当前项目"))?;
        if existing.locked
            || existing.status == SceneStatus::Generating
            || existing.pending_request_id.is_some()
        {
            return Err(StoryboardError::new("锁定或正在生成的分镜不能删除"));
        }
        let changed = transaction.execute(
            "DELETE FROM storyboard_scenes WHERE id = ?1 AND project_id = ?2",
            params![id, project_id],
        )?;
        if changed != 1 {
            return Err(StoryboardError::new("分镜不存在或不属于当前项目"));
        }
        normalize_order(&transaction, project_id)?;
        transaction.commit()?;
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

    pub fn apply_edit(
        &self,
        input: StoryboardEditInput,
    ) -> Result<Vec<SceneDraft>, StoryboardError> {
        self.ensure_project(&input.project_id)?;
        if input.scenes.len() > 200 {
            return Err(StoryboardError::new("单个项目最多 200 个分镜"));
        }
        let mut connection = self.connection()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current = list_with_connection(&transaction, &input.project_id)?;
        if current != input.expected {
            return Err(StoryboardError::new(
                "分镜已发生变化，请刷新后再试；当前内容未被覆盖",
            ));
        }
        let mut ids = HashSet::new();
        for scene in &input.scenes {
            validate_scene(scene)?;
            if scene.project_id != input.project_id || !ids.insert(scene.id.clone()) {
                return Err(StoryboardError::new("分镜项目或编号无效"));
            }
        }
        for existing in &current {
            let next = input.scenes.iter().find(|scene| scene.id == existing.id);
            let changed = next
                .map(|scene| {
                    let mut normalized = scene.clone();
                    normalized.order = existing.order;
                    normalized.updated_at = existing.updated_at.clone();
                    normalized != *existing
                })
                .unwrap_or(true);
            if changed
                && (existing.locked
                    || existing.status == SceneStatus::Generating
                    || existing.pending_request_id.is_some())
            {
                return Err(StoryboardError::new(
                    "锁定或正在生成的分镜不能拆分、合并或删除",
                ));
            }
        }
        for existing in &current {
            if !ids.contains(&existing.id) {
                transaction.execute(
                    "DELETE FROM storyboard_scenes WHERE id = ?1 AND project_id = ?2",
                    params![existing.id, input.project_id],
                )?;
            }
        }
        for (order, mut scene) in input.scenes.into_iter().enumerate() {
            scene.order = order as u32;
            scene.updated_at = now_iso();
            write_scene(&transaction, scene)?;
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

fn list_with_connection(
    connection: &Connection,
    project_id: &str,
) -> Result<Vec<SceneDraft>, StoryboardError> {
    let mut statement = connection.prepare(
        "SELECT id, project_id, order_index, title, purpose, source_refs_json,
                narration, narration_mode, ambient_sound, on_screen_text_json, visual_plan, generation_mode,
                target_duration_ms, asset_ids_json, selected_version_id, last_job_id,
                last_upscale_job_id, pending_request_id, generation_stage, status, quality, updated_at,
                locked, audio_intent, visual_intent_json, prompt_mode
         FROM storyboard_scenes WHERE project_id = ?1
         ORDER BY order_index ASC, id ASC",
    )?;
    let rows = statement.query_map([project_id], scene_from_row)?;
    rows.map(|row| row.map_err(StoryboardError::from)).collect()
}

fn same_authored_content(left: &SceneDraft, right: &SceneDraft) -> bool {
    left.id == right.id
        && left.project_id == right.project_id
        && left.order == right.order
        && left.title == right.title
        && left.purpose == right.purpose
        && left.source_refs == right.source_refs
        && left.narration == right.narration
        && left.narration_mode == right.narration_mode
        && left.ambient_sound == right.ambient_sound
        && left.on_screen_text == right.on_screen_text
        && left.visual_plan == right.visual_plan
        && left.visual_intent == right.visual_intent
        && left.prompt_mode == right.prompt_mode
        && left.audio_intent == right.audio_intent
        && left.generation_mode == right.generation_mode
        && left.target_duration_ms == right.target_duration_ms
        && left.asset_ids == right.asset_ids
        && left.quality == right.quality
}

fn write_scene(connection: &Connection, scene: SceneDraft) -> Result<SceneDraft, StoryboardError> {
    let source_refs = serde_json::to_string(&scene.source_refs)?;
    let on_screen_text = serde_json::to_string(&scene.on_screen_text)?;
    let asset_ids = serde_json::to_string(&scene.asset_ids)?;
    let visual_intent = serde_json::to_string(&scene.visual_intent)?;
    let changed = connection.execute(
            "INSERT INTO storyboard_scenes (
                id, project_id, order_index, title, purpose, source_refs_json, narration,
                narration_mode, ambient_sound, on_screen_text_json, visual_plan, generation_mode, target_duration_ms,
                asset_ids_json, selected_version_id, last_job_id, last_upscale_job_id,
                pending_request_id, generation_stage, status, quality, updated_at,
                locked, audio_intent, visual_intent_json, prompt_mode
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26
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
                updated_at = excluded.updated_at,
                locked = excluded.locked,
                audio_intent = excluded.audio_intent,
                visual_intent_json = excluded.visual_intent_json,
                prompt_mode = excluded.prompt_mode
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
                scene.locked,
                scene.audio_intent.as_str(),
                visual_intent,
                scene.prompt_mode.as_str(),
            ],
        )?;
    if changed != 1 {
        return Err(StoryboardError::new(
            "分镜 ID 已属于其他项目，不能移动到当前项目",
        ));
    }
    Ok(scene)
}

fn validate_scene(scene: &SceneDraft) -> Result<(), StoryboardError> {
    validate_identifier(&scene.id, "分镜 ID")?;
    if scene.title.trim().is_empty() {
        return Err(StoryboardError::new("分镜标题不能为空"));
    }
    if scene.title.chars().count() > 200 {
        return Err(StoryboardError::new("分镜标题不能超过 200 个字符"));
    }
    if !(4_000..=15_000).contains(&scene.target_duration_ms)
        || scene.target_duration_ms % 1_000 != 0
    {
        return Err(StoryboardError::new("分镜时长必须是 4 到 15 秒的整数"));
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
    let audio_intent: String = row.get(23)?;
    let visual_intent_json: String = row.get(24)?;
    let prompt_mode: String = row.get(25)?;

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
        visual_intent: serde_json::from_str(&visual_intent_json)
            .map_err(|error| conversion_error(24, error))?,
        prompt_mode: PromptMode::from_database(&prompt_mode)
            .map_err(|error| conversion_error(25, error))?,
        audio_intent: AudioIntent::from_database(&audio_intent)
            .map_err(|error| conversion_error(23, error))?,
        locked: row.get(22)?,
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
            visual_intent: VisualIntent::default(),
            prompt_mode: PromptMode::Quick,
            audio_intent: AudioIntent::Environment,
            locked: false,
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

    fn editable_scene(project_id: &str, id: &str, order: u32) -> SceneDraft {
        let mut scene = scene(project_id, id, order);
        scene.selected_version_id = None;
        scene.last_job_id = None;
        scene.last_upscale_job_id = None;
        scene.pending_request_id = None;
        scene.generation_stage = None;
        scene.status = SceneStatus::Draft;
        scene
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
    fn validates_integer_duration_range_and_normalizes_order_after_delete() {
        let (_directory, projects, storyboard) = storage();
        let project = project(&projects, "雷电");
        let mut invalid = editable_scene(&project.id, "invalid", 0);
        invalid.target_duration_ms = 3_500;
        assert!(storyboard
            .upsert(invalid)
            .expect_err("invalid duration")
            .to_string()
            .contains("4 到 15 秒"));

        let mut adjustable = editable_scene(&project.id, "adjustable", 0);
        adjustable.target_duration_ms = 8_000;
        storyboard.upsert(adjustable).expect("8 second scene");

        storyboard
            .upsert(editable_scene(&project.id, "a", 0))
            .expect("insert a");
        storyboard
            .upsert(editable_scene(&project.id, "b", 1))
            .expect("insert b");
        storyboard
            .upsert(editable_scene(&project.id, "c", 2))
            .expect("insert c");
        storyboard.delete(&project.id, "b").expect("delete b");
        let listed = storyboard.list(&project.id).expect("list scenes");
        assert_eq!(
            listed.iter().map(|scene| scene.order).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn applies_structural_edits_atomically_and_rejects_stale_snapshots() {
        let (_directory, projects, storyboard) = storage();
        let project = project(&projects, "雷电");
        storyboard
            .upsert(editable_scene(&project.id, "a", 0))
            .expect("insert a");
        storyboard
            .upsert(editable_scene(&project.id, "b", 1))
            .expect("insert b");
        let expected = storyboard.list(&project.id).expect("initial snapshot");
        let reordered = storyboard
            .apply_edit(StoryboardEditInput {
                project_id: project.id.clone(),
                expected: expected.clone(),
                scenes: vec![expected[1].clone(), expected[0].clone()],
            })
            .expect("atomic reorder");
        assert_eq!(
            reordered
                .iter()
                .map(|scene| scene.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );

        let error = storyboard
            .apply_edit(StoryboardEditInput {
                project_id: project.id.clone(),
                expected,
                scenes: Vec::new(),
            })
            .expect_err("stale snapshot must fail");
        assert!(error.to_string().contains("已发生变化"));
        assert_eq!(storyboard.list(&project.id).expect("unchanged"), reordered);
    }

    #[test]
    fn structural_edit_does_not_remove_locked_scene() {
        let (_directory, projects, storyboard) = storage();
        let project = project(&projects, "雷电");
        let mut locked = editable_scene(&project.id, "locked", 0);
        locked.locked = true;
        storyboard.upsert(locked).expect("insert locked scene");
        let expected = storyboard.list(&project.id).expect("locked snapshot");
        let error = storyboard
            .apply_edit(StoryboardEditInput {
                project_id: project.id.clone(),
                expected: expected.clone(),
                scenes: Vec::new(),
            })
            .expect_err("locked scene must survive");
        assert!(error.to_string().contains("锁定"));
        assert_eq!(storyboard.list(&project.id).expect("unchanged"), expected);
    }
}
