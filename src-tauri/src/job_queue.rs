use crate::{
    service::{ServiceJob, SubmitServiceJobInput},
    storage::ProjectStorage,
};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;
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
    pub worker_id: Option<String>,
    pub lease_expires_at: Option<String>,
    pub attempt: u32,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
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
                worker_id         TEXT,
                lease_expires_at  TEXT,
                attempt           INTEGER NOT NULL DEFAULT 0,
                error_code        TEXT,
                error_message     TEXT,
                created_at        TEXT NOT NULL,
                updated_at        TEXT NOT NULL,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_generation_jobs_project_updated
                ON generation_jobs(project_id, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_generation_jobs_dispatch
                ON generation_jobs(status, lease_expires_at, created_at);
            ",
            )
            .map_err(database_error)?;
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
                  request_json, status, progress, attempt, error_code, error_message, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, '{}', ?7, ?8, 1, ?9, ?10, ?11, ?11)
                 ON CONFLICT(client_request_id) DO UPDATE SET
                   remote_job_id=excluded.remote_job_id,
                   status=excluded.status,
                   progress=excluded.progress,
                   error_code=excluded.error_code,
                   error_message=excluded.error_message,
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
                    job.error_code,
                    job.error_message,
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
                "UPDATE generation_jobs SET status=?2, progress=?3, error_code=?4,
             error_message=?5,
             worker_id=CASE WHEN ?2 IN ('completed','failed','cancelled','interrupted') THEN NULL ELSE worker_id END,
             lease_expires_at=CASE WHEN ?2 IN ('completed','failed','cancelled','interrupted') THEN NULL ELSE lease_expires_at END,
             updated_at=?6 WHERE remote_job_id=?1",
                params![
                    job.id,
                    job.status,
                    job.progress,
                    job.error_code,
                    job.error_message,
                    now_iso()
                ],
            )
            .map_err(database_error)?;
        if changed == 0 {
            return self.record_remote(job);
        }
        self.find_by_request(&job.client_request_id)
    }

    pub fn mark_local_complete(&self, remote_job_id: &str) -> QueueResult<()> {
        self.connection()?
            .execute(
                "UPDATE generation_jobs SET status='completed_local', progress=1,
             worker_id=NULL, lease_expires_at=NULL, updated_at=?2 WHERE remote_job_id=?1",
                params![remote_job_id, now_iso()],
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
                          workflow_id, status, progress, worker_id, lease_expires_at,
                          attempt, error_code, error_message, created_at, updated_at
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

    fn find_by_request(&self, request_id: &str) -> QueueResult<LocalJob> {
        self.connection()?
            .query_row(
                "SELECT client_request_id, remote_job_id, project_id, scene_id, kind,
                    workflow_id, status, progress, worker_id, lease_expires_at,
                    attempt, error_code, error_message, created_at, updated_at
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
        worker_id: row.get(8)?,
        lease_expires_at: row.get(9)?,
        attempt: row.get(10)?,
        error_code: row.get(11)?,
        error_message: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
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
                on_screen_text: Vec::new(),
                visual_plan: "画面".into(),
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
            error_code: None,
            error_message: None,
            status_detail: None,
            created_at: now_iso(),
            updated_at: now_iso(),
            result_manifest: None,
        };
        let stored = queue.record_remote(&job).expect("remote");
        assert_eq!(stored.remote_job_id.as_deref(), Some("remote-1"));
        assert_eq!(stored.attempt, 1);
        assert_eq!(queue.list(Some(&stored.project_id)).expect("list").len(), 1);
        queue.mark_local_complete("remote-1").expect("complete");
        assert_eq!(
            queue.find_by_request("request-1").expect("find").status,
            "completed_local"
        );
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
