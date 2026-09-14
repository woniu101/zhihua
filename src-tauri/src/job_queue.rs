use crate::{
    service::{ServiceJob, SubmitServiceJobInput},
    storage::ProjectStorage,
};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::Value;
use std::{fmt, path::PathBuf, time::Duration};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobQueueError {
    pub code: String,
    pub message: String,
}

impl JobQueueError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for JobQueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

type QueueResult<T> = Result<T, JobQueueError>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalJob {
    pub client_request_id: String,
    pub remote_job_id: Option<String>,
    pub project_id: String,
    pub scene_id: String,
    pub kind: String,
    pub workflow_id: String,
    pub status: String,
    pub progress: f64,
    pub progress_stage: String,
    pub progress_measured: bool,
    pub progress_current: Option<u64>,
    pub progress_total: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub worker_id: Option<String>,
    pub lease_expires_at: Option<String>,
    pub attempt: u32,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub status_detail: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone)]
pub struct JobQueueStorage {
    database_path: PathBuf,
}

impl JobQueueStorage {
    pub fn initialize(projects: ProjectStorage) -> QueueResult<Self> {
        let storage = Self {
            database_path: projects.info().database_path,
        };
        let connection = storage.connection()?;
        connection
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS generation_jobs (
                client_request_id TEXT PRIMARY KEY NOT NULL,
                remote_job_id     TEXT UNIQUE,
                project_id        TEXT NOT NULL,
                scene_id          TEXT NOT NULL,
                kind              TEXT NOT NULL,
                workflow_id       TEXT NOT NULL,
                request_json      TEXT NOT NULL,
                status            TEXT NOT NULL,
                progress          REAL NOT NULL DEFAULT 0,
                progress_stage    TEXT NOT NULL DEFAULT 'queued',
                progress_measured INTEGER NOT NULL DEFAULT 0,
                progress_current  INTEGER,
                progress_total    INTEGER,
                eta_seconds       INTEGER,
                worker_id         TEXT,
                lease_expires_at  TEXT,
                attempt           INTEGER NOT NULL DEFAULT 0,
                error_code        TEXT,
                error_message     TEXT,
                created_at        TEXT NOT NULL,
                updated_at        TEXT NOT NULL,
                status_detail     TEXT,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_generation_jobs_project_updated
                ON generation_jobs(project_id, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_generation_jobs_dispatch
                ON generation_jobs(status, lease_expires_at, created_at);
            ",
            )
            .map_err(database_error)?;
        let has_status_detail = {
            let mut statement = connection
                .prepare("PRAGMA table_info(generation_jobs)")
                .map_err(database_error)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(database_error)?;
            let found = rows
                .filter_map(Result::ok)
                .any(|column| column == "status_detail");
            found
        };
        if !has_status_detail {
            connection
                .execute(
                    "ALTER TABLE generation_jobs ADD COLUMN status_detail TEXT",
                    [],
                )
                .map_err(database_error)?;
        }
        let has_scene_foreign_key = {
            let mut statement = connection
                .prepare("PRAGMA foreign_key_list(generation_jobs)")
                .map_err(database_error)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(2))
                .map_err(database_error)?;
            let found = rows
                .filter_map(Result::ok)
                .any(|table| table == "storyboard_scenes");
            found
        };
        if has_scene_foreign_key {
            connection
                .pragma_update(None, "foreign_keys", "OFF")
                .map_err(database_error)?;
            let migration = connection.execute_batch(
                "
                    BEGIN IMMEDIATE;
                    ALTER TABLE generation_jobs RENAME TO generation_jobs_legacy;
                    CREATE TABLE generation_jobs (
                        client_request_id TEXT PRIMARY KEY NOT NULL,
                        remote_job_id     TEXT UNIQUE,
                        project_id        TEXT NOT NULL,
                        scene_id          TEXT NOT NULL,
                        kind              TEXT NOT NULL,
                        workflow_id       TEXT NOT NULL,
                        request_json      TEXT NOT NULL,
                        status            TEXT NOT NULL,
                        progress          REAL NOT NULL DEFAULT 0,
                        worker_id         TEXT,
                        lease_expires_at  TEXT,
                        attempt           INTEGER NOT NULL DEFAULT 0,
                        error_code        TEXT,
                        error_message     TEXT,
                        created_at        TEXT NOT NULL,
                        updated_at        TEXT NOT NULL,
                        status_detail     TEXT,
                        FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
                    );
                    INSERT INTO generation_jobs
                    SELECT * FROM generation_jobs_legacy;
                    DROP TABLE generation_jobs_legacy;
                    CREATE INDEX idx_generation_jobs_project_updated
                        ON generation_jobs(project_id, updated_at DESC);
                    CREATE INDEX idx_generation_jobs_dispatch
                        ON generation_jobs(status, lease_expires_at, created_at);
                    COMMIT;
                    ",
            );
            let restore = connection
                .pragma_update(None, "foreign_keys", "ON")
                .map_err(database_error);
            migration.map_err(database_error)?;
            restore?;
        }
        let columns = {
            let mut statement = connection
                .prepare("PRAGMA table_info(generation_jobs)")
                .map_err(database_error)?;
            let result = statement
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(database_error)?
                .filter_map(Result::ok)
                .collect::<Vec<_>>();
            result
        };
        for (column, definition) in [
            ("progress_stage", "TEXT NOT NULL DEFAULT 'queued'"),
            ("progress_measured", "INTEGER NOT NULL DEFAULT 0"),
            ("progress_current", "INTEGER"),
            ("progress_total", "INTEGER"),
            ("eta_seconds", "INTEGER"),
        ] {
            if !columns.iter().any(|item| item == column) {
                connection
                    .execute(
                        &format!("ALTER TABLE generation_jobs ADD COLUMN {column} {definition}"),
                        [],
                    )
                    .map_err(database_error)?;
            }
        }
        storage.release_expired_leases()?;
        Ok(storage)
    }

    fn connection(&self) -> QueueResult<Connection> {
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

    pub fn stage(&self, input: &SubmitServiceJobInput) -> QueueResult<LocalJob> {
        let now = now_iso();
        let request_json = serde_json::to_string(&input.parameters).map_err(|error| {
            JobQueueError::new("SERIALIZE_ERROR", format!("任务参数无法保存：{error}"))
        })?;
        self.connection()?
            .execute(
                "INSERT INTO generation_jobs
             (client_request_id, project_id, scene_id, kind, workflow_id, request_json,
              status, progress, attempt, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending_submit', 0, 0, ?7, ?7)
             ON CONFLICT(client_request_id) DO UPDATE SET
               updated_at=excluded.updated_at
             WHERE generation_jobs.remote_job_id IS NULL
               AND generation_jobs.status IN ('pending_submit','submit_failed')",
                params![
                    input.client_request_id,
                    input.project_id,
                    input.scene_id,
                    input.kind,
                    input.workflow_id,
                    request_json,
                    now
                ],
            )
            .map_err(database_error)?;
        self.find_by_request(&input.client_request_id)
    }

    pub fn record_remote(&self, job: &ServiceJob) -> QueueResult<LocalJob> {
        let now = now_iso();
        self.connection()?
            .execute(
                "INSERT INTO generation_jobs
                 (client_request_id, remote_job_id, project_id, scene_id, kind, workflow_id,
                  request_json, status, progress, progress_stage, progress_measured,
                  progress_current, progress_total, eta_seconds, attempt, error_code,
                  error_message, status_detail, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, '{}', ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                         1, ?14, ?15, ?16, ?17, ?17)
                 ON CONFLICT(client_request_id) DO UPDATE SET
                   remote_job_id=excluded.remote_job_id,
                   status=excluded.status,
                   progress=excluded.progress,
                   progress_stage=excluded.progress_stage,
                   progress_measured=excluded.progress_measured,
                   progress_current=excluded.progress_current,
                   progress_total=excluded.progress_total,
                   eta_seconds=excluded.eta_seconds,
                   error_code=excluded.error_code,
                   error_message=excluded.error_message,
                   status_detail=excluded.status_detail,
                   attempt=CASE WHEN generation_jobs.remote_job_id IS NULL
                     THEN generation_jobs.attempt + 1 ELSE generation_jobs.attempt END,
                   updated_at=excluded.updated_at",
                params![
                    job.client_request_id,
                    job.id,
                    job.project_id,
                    job.scene_id,
                    job.kind,
                    job.workflow_id,
                    job.status,
                    job.progress,
                    job.progress_stage,
                    job.progress_measured,
                    job.progress_current,
                    job.progress_total,
                    job.eta_seconds,
                    job.error_code,
                    job.error_message,
                    job.status_detail,
                    now
                ],
            )
            .map_err(database_error)?;
        self.find_by_request(&job.client_request_id)
    }

    pub fn record_submit_failure(
        &self,
        request_id: &str,
        code: &str,
        message: &str,
    ) -> QueueResult<()> {
        self.connection()?
            .execute(
                "UPDATE generation_jobs SET status='submit_failed', error_code=?2,
             error_message=?3, attempt=attempt+1, updated_at=?4
             WHERE client_request_id=?1 AND remote_job_id IS NULL",
                params![request_id, code, message, now_iso()],
            )
            .map_err(database_error)?;
        Ok(())
    }

    pub fn sync(&self, job: &ServiceJob) -> QueueResult<LocalJob> {
        let changed = self
            .connection()?
            .execute(
                "UPDATE generation_jobs SET status=?2, progress=?3, progress_stage=?4,
             progress_measured=?5, progress_current=?6, progress_total=?7, eta_seconds=?8,
             error_code=?9, error_message=?10, status_detail=?11,
             lease_expires_at=CASE WHEN ?2 IN ('completed','failed','cancelled','interrupted') THEN NULL ELSE lease_expires_at END,
             updated_at=?12 WHERE remote_job_id=?1 AND status!='downloading'",
                params![
                    job.id,
                    job.status,
                    job.progress,
                    job.progress_stage,
                    job.progress_measured,
                    job.progress_current,
                    job.progress_total,
                    job.eta_seconds,
                    job.error_code,
                    job.error_message,
                    job.status_detail,
                    now_iso()
                ],
            )
            .map_err(database_error)?;
        if changed == 0 {
            if let Ok(existing) = self.find_by_remote(&job.id) {
                return Ok(existing);
            }
            return self.record_remote(job);
        }
        self.find_by_request(&job.client_request_id)
    }

    pub fn mark_local_complete(&self, remote_job_id: &str) -> QueueResult<()> {
        self.connection()?
            .execute(
                "UPDATE generation_jobs SET status='completed_local', progress=1,
             progress_stage='completed', progress_measured=1,
             progress_current=1, progress_total=1, eta_seconds=0,
             error_code=NULL, error_message=NULL, status_detail='结果已保存到本地',
             lease_expires_at=NULL, updated_at=?2 WHERE remote_job_id=?1",
                params![remote_job_id, now_iso()],
            )
            .map_err(database_error)?;
        Ok(())
    }

    pub fn mark_downloading(
        &self,
        remote_job_id: &str,
        current: u64,
        total: u64,
    ) -> QueueResult<()> {
        if total == 0 || current > total {
            return Err(JobQueueError::new("INVALID_PROGRESS", "下载进度数据无效"));
        }
        self.connection()?
            .execute(
                "UPDATE generation_jobs SET status='downloading',
                 progress=CAST(?2 AS REAL)/CAST(?3 AS REAL), progress_stage='downloading',
                 progress_measured=1, progress_current=?2, progress_total=?3,
                 eta_seconds=NULL, error_code=NULL, error_message=NULL,
                 status_detail='正在续传并校验生成结果', updated_at=?4
                 WHERE remote_job_id=?1",
                params![remote_job_id, current, total, now_iso()],
            )
            .map_err(database_error)?;
        Ok(())
    }

    pub fn mark_download_retry(&self, remote_job_id: &str, message: &str) -> QueueResult<()> {
        self.connection()?
            .execute(
                "UPDATE generation_jobs SET status='completed', progress=1,
                 progress_stage='completed', progress_measured=1,
                 progress_current=1, progress_total=1, eta_seconds=0,
                 error_code='DOWNLOAD_INTERRUPTED', error_message=?2,
                 status_detail='结果仍在远端，可继续下载', updated_at=?3
                 WHERE remote_job_id=?1",
                params![remote_job_id, message, now_iso()],
            )
            .map_err(database_error)?;
        Ok(())
    }

    pub fn claim_for_worker(
        &self,
        request_id: &str,
        worker_id: &str,
        lease_seconds: u32,
    ) -> QueueResult<LocalJob> {
        let worker_id = worker_id.trim();
        if worker_id.is_empty() || !(30..=3_600).contains(&lease_seconds) {
            return Err(JobQueueError::new(
                "INVALID_LEASE",
                "worker 标识为空或租约时长不在 30～3600 秒范围内",
            ));
        }
        let now = Utc::now();
        let now_text = now.to_rfc3339_opts(SecondsFormat::Millis, true);
        let expires = (now + ChronoDuration::seconds(lease_seconds as i64))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let changed = self
            .connection()?
            .execute(
                "UPDATE generation_jobs SET worker_id=?2, lease_expires_at=?3,
                 status='leased', updated_at=?4
                 WHERE client_request_id=?1 AND remote_job_id IS NULL
                   AND status IN ('pending_submit','submit_failed','leased')
                   AND (worker_id IS NULL OR worker_id=?2 OR lease_expires_at<=?4)",
                params![request_id, worker_id, expires, now_text],
            )
            .map_err(database_error)?;
        if changed != 1 {
            return Err(JobQueueError::new(
                "LEASE_UNAVAILABLE",
                "任务已被其他 worker 租用或不再等待提交",
            ));
        }
        self.find_by_request(request_id)
    }

    pub fn release_expired_leases(&self) -> QueueResult<usize> {
        self.connection()?
            .execute(
                "UPDATE generation_jobs SET worker_id=NULL, lease_expires_at=NULL,
                 status='pending_submit', updated_at=?1
                 WHERE remote_job_id IS NULL AND lease_expires_at IS NOT NULL
                   AND lease_expires_at<=?1",
                [now_iso()],
            )
            .map_err(database_error)
    }

    pub fn list(&self, project_id: Option<&str>) -> QueueResult<Vec<LocalJob>> {
        let connection = self.connection()?;
        let sql = "SELECT client_request_id, remote_job_id, project_id, scene_id, kind,
                          workflow_id, status, progress, progress_stage, progress_measured,
                          progress_current, progress_total, eta_seconds, worker_id, lease_expires_at,
                          attempt, error_code, error_message, status_detail, created_at, updated_at
                   FROM generation_jobs";
        let mut statement = if project_id.is_some() {
            connection
                .prepare(&format!(
                    "{sql} WHERE project_id=?1 ORDER BY created_at ASC"
                ))
                .map_err(database_error)?
        } else {
            connection
                .prepare(&format!("{sql} ORDER BY created_at ASC"))
                .map_err(database_error)?
        };
        let map = |row: &rusqlite::Row<'_>| local_job_from_row(row);
        let rows = match project_id {
            Some(project_id) => statement
                .query_map([project_id], map)
                .map_err(database_error)?,
            None => statement.query_map([], map).map_err(database_error)?,
        };
        rows.collect::<Result<Vec<_>, _>>().map_err(database_error)
    }

    pub fn has_unsettled_remote_jobs_for_worker(&self, worker_id: &str) -> QueueResult<bool> {
        let count: i64 = self
            .connection()?
            .query_row(
                "SELECT COUNT(*) FROM generation_jobs
                 WHERE worker_id=?1 AND remote_job_id IS NOT NULL
                   AND status NOT IN ('completed_local','failed','cancelled','interrupted')",
                [worker_id],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        Ok(count > 0)
    }

    pub fn request_parameters(&self, remote_job_id: &str) -> QueueResult<Value> {
        let request_json = self
            .connection()?
            .query_row(
                "SELECT request_json FROM generation_jobs WHERE remote_job_id=?1",
                [remote_job_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(database_error)?;
        serde_json::from_str(&request_json).map_err(|error| {
            JobQueueError::new(
                "DESERIALIZE_ERROR",
                format!("本地任务生成参数无法读取：{error}"),
            )
        })
    }

    pub fn find_by_remote(&self, remote_job_id: &str) -> QueueResult<LocalJob> {
        self.connection()?
            .query_row(
                "SELECT client_request_id, remote_job_id, project_id, scene_id, kind,
                    workflow_id, status, progress, progress_stage, progress_measured,
                    progress_current, progress_total, eta_seconds, worker_id, lease_expires_at,
                    attempt, error_code, error_message, status_detail, created_at, updated_at
             FROM generation_jobs WHERE remote_job_id=?1",
                [remote_job_id],
                local_job_from_row,
            )
            .map_err(database_error)
    }

    fn find_by_request(&self, request_id: &str) -> QueueResult<LocalJob> {
        self.connection()?
            .query_row(
                "SELECT client_request_id, remote_job_id, project_id, scene_id, kind,
                    workflow_id, status, progress, progress_stage, progress_measured,
                    progress_current, progress_total, eta_seconds, worker_id, lease_expires_at,
                    attempt, error_code, error_message, status_detail, created_at, updated_at
             FROM generation_jobs WHERE client_request_id=?1",
                [request_id],
                local_job_from_row,
            )
            .map_err(database_error)
    }
}

fn local_job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalJob> {
    Ok(LocalJob {
        client_request_id: row.get(0)?,
        remote_job_id: row.get(1)?,
        project_id: row.get(2)?,
        scene_id: row.get(3)?,
        kind: row.get(4)?,
        workflow_id: row.get(5)?,
        status: row.get(6)?,
        progress: row.get(7)?,
        progress_stage: row.get(8)?,
        progress_measured: row.get(9)?,
        progress_current: row.get(10)?,
        progress_total: row.get(11)?,
        eta_seconds: row.get(12)?,
        worker_id: row.get(13)?,
        lease_expires_at: row.get(14)?,
        attempt: row.get(15)?,
        error_code: row.get(16)?,
        error_message: row.get(17)?,
        status_detail: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
    })
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
fn database_error(error: rusqlite::Error) -> JobQueueError {
    JobQueueError::new("DATABASE_ERROR", format!("本地任务队列操作失败：{error}"))
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
    use serde_json::json;

    fn setup() -> (tempfile::TempDir, JobQueueStorage, String, String) {
        let directory = tempfile::tempdir().expect("temporary directory");
        let projects = ProjectStorage::initialize(
            directory.path().join("db.sqlite3"),
            directory.path().join("projects"),
        )
        .expect("projects");
        let project = projects
            .create_project(CreateProjectInput {
                title: "任务".into(),
                audience: None,
                target_duration_sec: Some(5),
            })
            .expect("project");
        let storyboards = StoryboardStorage::initialize(projects.clone()).expect("storyboards");
        let scene = storyboards
            .upsert(SceneDraft {
                id: "scene-1".into(),
                project_id: project.id.clone(),
                order: 0,
                title: "镜头".into(),
                purpose: String::new(),
                source_refs: Vec::new(),
                narration: "旁白".into(),
                narration_mode: crate::storyboard::NarrationMode::Tts,
                ambient_sound: "环境声".into(),
                on_screen_text: Vec::new(),
                visual_plan: "画面".into(),
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
        let queue = JobQueueStorage::initialize(projects).expect("queue");
        (directory, queue, project.id, scene.id)
    }

    #[test]
    fn stages_and_reconciles_an_idempotent_remote_job() {
        let (_directory, queue, project_id, scene_id) = setup();
        let input = SubmitServiceJobInput {
            client_request_id: "request-1".into(),
            project_id: project_id.clone(),
            scene_id: scene_id.clone(),
            kind: "video_candidate".into(),
            workflow_id: "h3-t2v-turbo-v1".into(),
            parameters: json!({"seed": 7}),
        };
        queue.stage(&input).expect("stage");
        queue.stage(&input).expect("stage idempotently");
        queue
            .claim_for_worker("request-1", "instance:worker-one:gpu:0", 60)
            .expect("claim worker");
        let job = ServiceJob {
            id: "remote-1".into(),
            client_request_id: input.client_request_id.clone(),
            project_id,
            scene_id,
            kind: input.kind,
            workflow_id: input.workflow_id,
            status: "running".into(),
            prompt_id: Some("prompt-1".into()),
            progress: 0.4,
            progress_stage: "model_inference".into(),
            progress_measured: true,
            progress_current: Some(4),
            progress_total: Some(10),
            eta_seconds: Some(30),
            error_code: None,
            error_message: None,
            status_detail: Some("ComfyUI is executing the prompt".into()),
            created_at: now_iso(),
            updated_at: now_iso(),
            result_manifest: None,
        };
        let stored = queue.record_remote(&job).expect("remote");
        assert_eq!(stored.remote_job_id.as_deref(), Some("remote-1"));
        assert_eq!(
            stored.status_detail.as_deref(),
            Some("ComfyUI is executing the prompt")
        );
        assert_eq!(stored.attempt, 1);
        assert_eq!(
            stored.worker_id.as_deref(),
            Some("instance:worker-one:gpu:0")
        );
        assert_eq!(queue.list(Some(&stored.project_id)).expect("list").len(), 1);
        let mut completed = job.clone();
        completed.status = "completed".into();
        let synced = queue.sync(&completed).expect("sync completed job");
        assert!(queue
            .has_unsettled_remote_jobs_for_worker("instance:worker-one:gpu:0")
            .expect("unsettled result"));
        assert_eq!(
            synced.worker_id.as_deref(),
            Some("instance:worker-one:gpu:0")
        );
        assert_eq!(
            queue
                .find_by_remote("remote-1")
                .expect("find remote")
                .worker_id
                .as_deref(),
            Some("instance:worker-one:gpu:0")
        );
        queue
            .mark_downloading("remote-1", 4, 10)
            .expect("mark measured download");
        let downloading = queue
            .sync(&completed)
            .expect("remote polling must not overwrite a local transfer");
        assert_eq!(downloading.status, "downloading");
        assert!(downloading.progress_measured);
        assert_eq!(downloading.progress_current, Some(4));
        assert_eq!(downloading.progress_total, Some(10));
        queue
            .mark_download_retry("remote-1", "desktop restarted")
            .expect("make interrupted download retryable");
        let retryable = queue.find_by_remote("remote-1").expect("retryable result");
        assert_eq!(retryable.status, "completed");
        assert_eq!(
            retryable.error_code.as_deref(),
            Some("DOWNLOAD_INTERRUPTED")
        );
        queue.mark_local_complete("remote-1").expect("complete");
        assert!(!queue
            .has_unsettled_remote_jobs_for_worker("instance:worker-one:gpu:0")
            .expect("settled result"));
        let local = queue.find_by_request("request-1").expect("find");
        assert_eq!(local.status, "completed_local");
        assert_eq!(
            local.worker_id.as_deref(),
            Some("instance:worker-one:gpu:0")
        );
    }

    #[test]
    fn reopens_a_pending_job_after_an_abrupt_desktop_exit() {
        let (directory, queue, project_id, scene_id) = setup();
        let input = SubmitServiceJobInput {
            client_request_id: "request-after-exit".into(),
            project_id,
            scene_id,
            kind: "video_candidate".into(),
            workflow_id: "h3-t2v-turbo-v1".into(),
            parameters: json!({"seed": 19}),
        };
        queue.stage(&input).expect("persist pending job");
        drop(queue);

        let projects = ProjectStorage::initialize(
            directory.path().join("db.sqlite3"),
            directory.path().join("projects"),
        )
        .expect("reopen projects after process exit");
        let reopened = JobQueueStorage::initialize(projects).expect("reopen durable queue");
        let recovered = reopened
            .find_by_request("request-after-exit")
            .expect("recover pending job");
        assert_eq!(recovered.status, "pending_submit");
        assert_eq!(recovered.workflow_id, "h3-t2v-turbo-v1");
    }

    #[test]
    fn accepts_project_level_image_jobs_without_a_storyboard_scene() {
        let (_directory, queue, project_id, _scene_id) = setup();
        let input = SubmitServiceJobInput {
            client_request_id: "image-request-1".into(),
            project_id,
            scene_id: "project-assets".into(),
            kind: "image_generation".into(),
            workflow_id: "qwen-image-generate-v1".into(),
            parameters: serde_json::json!({"prompt": "一张科普插画"}),
        };
        let stored = queue.stage(&input).expect("stage project-level image job");
        assert_eq!(stored.scene_id, "project-assets");
    }

    #[test]
    fn prevents_two_workers_from_claiming_the_same_pending_request() {
        let (_directory, queue, project_id, scene_id) = setup();
        queue
            .stage(&SubmitServiceJobInput {
                client_request_id: "request-lease".into(),
                project_id,
                scene_id,
                kind: "video_candidate".into(),
                workflow_id: "h3-t2v-turbo-v1".into(),
                parameters: json!({}),
            })
            .expect("stage");
        let claimed = queue
            .claim_for_worker("request-lease", "worker-a", 60)
            .expect("first claim");
        assert_eq!(claimed.worker_id.as_deref(), Some("worker-a"));
        assert_eq!(claimed.status, "leased");
        let error = queue
            .claim_for_worker("request-lease", "worker-b", 60)
            .expect_err("second worker must not claim an active lease");
        assert_eq!(error.code, "LEASE_UNAVAILABLE");
        assert_eq!(queue.release_expired_leases().expect("release"), 0);
    }
}
