use crate::comp_share::{CompShareInstance, CompSharePowerState, CompShareRunningMode};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, path::PathBuf, str::FromStr, time::Duration};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputeControlError {
    pub code: String,
    pub message: String,
}

impl ComputeControlError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ComputeControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ComputeControlError {}

impl From<rusqlite::Error> for ComputeControlError {
    fn from(error: rusqlite::Error) -> Self {
        Self::new(
            "COMPUTE_DATABASE_ERROR",
            format!("算力数据库操作失败：{error}"),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeInstanceRole {
    Primary,
    Elastic,
    UserManaged,
    Test,
}

impl ComputeInstanceRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Elastic => "elastic",
            Self::UserManaged => "user_managed",
            Self::Test => "test",
        }
    }
}

impl FromStr for ComputeInstanceRole {
    type Err = ComputeControlError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "primary" => Ok(Self::Primary),
            "elastic" => Ok(Self::Elastic),
            "user_managed" => Ok(Self::UserManaged),
            "test" => Ok(Self::Test),
            _ => Err(ComputeControlError::new(
                "INVALID_COMPUTE_ROLE",
                format!("未知的实例角色：{value}"),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeInstanceOwnership {
    ZhihuaManaged,
    UserManaged,
}

impl FromStr for ComputeInstanceOwnership {
    type Err = ComputeControlError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "zhihua_managed" => Ok(Self::ZhihuaManaged),
            "user_managed" => Ok(Self::UserManaged),
            _ => Err(ComputeControlError::new(
                "INVALID_COMPUTE_OWNERSHIP",
                format!("未知的实例所有权：{value}"),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeCleanupPolicy {
    Retain,
    ReleaseWhenIdle,
}

impl ComputeCleanupPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Retain => "retain",
            Self::ReleaseWhenIdle => "release_when_idle",
        }
    }
}

impl FromStr for ComputeCleanupPolicy {
    type Err = ComputeControlError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "retain" => Ok(Self::Retain),
            "release_when_idle" => Ok(Self::ReleaseWhenIdle),
            _ => Err(ComputeControlError::new(
                "INVALID_CLEANUP_POLICY",
                format!("未知的实例清理策略：{value}"),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeLifecycleState {
    Discovered,
    Creating,
    Starting,
    Preparing,
    Idle,
    Busy,
    Draining,
    Stopping,
    Retained,
    Terminating,
    Terminated,
    Unknown,
    Error,
}

impl ComputeLifecycleState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::Creating => "creating",
            Self::Starting => "starting",
            Self::Preparing => "preparing",
            Self::Idle => "idle",
            Self::Busy => "busy",
            Self::Draining => "draining",
            Self::Stopping => "stopping",
            Self::Retained => "retained",
            Self::Terminating => "terminating",
            Self::Terminated => "terminated",
            Self::Unknown => "unknown",
            Self::Error => "error",
        }
    }
}

impl FromStr for ComputeLifecycleState {
    type Err = ComputeControlError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "discovered" => Ok(Self::Discovered),
            "creating" => Ok(Self::Creating),
            "starting" => Ok(Self::Starting),
            "preparing" => Ok(Self::Preparing),
            "idle" => Ok(Self::Idle),
            "busy" => Ok(Self::Busy),
            "draining" => Ok(Self::Draining),
            "stopping" => Ok(Self::Stopping),
            "retained" => Ok(Self::Retained),
            "terminating" => Ok(Self::Terminating),
            "terminated" => Ok(Self::Terminated),
            "unknown" => Ok(Self::Unknown),
            "error" => Ok(Self::Error),
            _ => Err(ComputeControlError::new(
                "INVALID_LIFECYCLE_STATE",
                format!("未知的实例生命周期状态：{value}"),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeOperationAction {
    Create,
    StartGpu,
    StartNoGpu,
    Stop,
    RefreshRetention,
    Terminate,
}

impl ComputeOperationAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::StartGpu => "start_gpu",
            Self::StartNoGpu => "start_no_gpu",
            Self::Stop => "stop",
            Self::RefreshRetention => "refresh_retention",
            Self::Terminate => "terminate",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeOperationStatus {
    Pending,
    Succeeded,
    Failed,
    Unknown,
}

impl ComputeOperationStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedComputeInstance {
    pub instance_id: String,
    pub name: Option<String>,
    pub region: String,
    pub zone: String,
    pub project_id: Option<String>,
    pub role: ComputeInstanceRole,
    pub ownership: ComputeInstanceOwnership,
    pub cleanup_policy: ComputeCleanupPolicy,
    pub lifecycle_state: ComputeLifecycleState,
    pub platform_state: String,
    pub running_mode: String,
    pub gpu_type: Option<String>,
    pub gpu_count: Option<u32>,
    pub image_id: Option<String>,
    pub release_time: Option<i64>,
    pub stop_time: Option<i64>,
    pub stop_scheduler_time: Option<i64>,
    pub instance_price: Option<f64>,
    pub disk_price: Option<f64>,
    pub current_job_id: Option<String>,
    pub last_synced_at: String,
    pub missing_since: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComputeOperation {
    pub id: String,
    pub idempotency_key: String,
    pub instance_id: Option<String>,
    pub action: ComputeOperationAction,
    pub status: ComputeOperationStatus,
    pub request_uuid: Option<String>,
    pub error_message: Option<String>,
    pub payload: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseEligibility {
    pub allowed: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ComputeControlStore {
    database_path: PathBuf,
}

impl ComputeControlStore {
    pub fn initialize(database_path: PathBuf) -> Result<Self, ComputeControlError> {
        let store = Self { database_path };
        let connection = store.connection()?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS compute_instances (
                instance_id             TEXT PRIMARY KEY NOT NULL,
                name                    TEXT,
                region                  TEXT NOT NULL,
                zone                    TEXT NOT NULL,
                project_id              TEXT,
                role                    TEXT NOT NULL,
                ownership               TEXT NOT NULL,
                cleanup_policy          TEXT NOT NULL,
                lifecycle_state         TEXT NOT NULL,
                platform_state          TEXT NOT NULL,
                running_mode            TEXT NOT NULL,
                gpu_type                TEXT,
                gpu_count               INTEGER,
                image_id                TEXT,
                release_time            INTEGER,
                stop_time               INTEGER,
                stop_scheduler_time     INTEGER,
                instance_price          REAL,
                disk_price              REAL,
                current_job_id          TEXT,
                last_synced_at          TEXT NOT NULL,
                missing_since           TEXT,
                created_at              TEXT NOT NULL,
                updated_at              TEXT NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_compute_single_primary
                ON compute_instances(role) WHERE role = 'primary';
            CREATE INDEX IF NOT EXISTS idx_compute_instances_cleanup
                ON compute_instances(ownership, cleanup_policy, lifecycle_state);

            CREATE TABLE IF NOT EXISTS compute_operations (
                id                      TEXT PRIMARY KEY NOT NULL,
                idempotency_key         TEXT NOT NULL UNIQUE,
                instance_id             TEXT,
                action                  TEXT NOT NULL,
                status                  TEXT NOT NULL,
                request_uuid            TEXT,
                error_message           TEXT,
                payload_json            TEXT NOT NULL DEFAULT '{}',
                created_at              TEXT NOT NULL,
                updated_at              TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_compute_operations_pending
                ON compute_operations(status, updated_at);

            CREATE TABLE IF NOT EXISTS worker_leases (
                worker_id               TEXT PRIMARY KEY NOT NULL,
                instance_id             TEXT NOT NULL,
                job_id                  TEXT NOT NULL UNIQUE,
                lease_expires_at        TEXT NOT NULL,
                created_at              TEXT NOT NULL,
                updated_at              TEXT NOT NULL,
                FOREIGN KEY(instance_id) REFERENCES compute_instances(instance_id)
                    ON DELETE RESTRICT
            );
            CREATE INDEX IF NOT EXISTS idx_worker_leases_instance
                ON worker_leases(instance_id, lease_expires_at);
            ",
        )?;
        Ok(store)
    }

    fn connection(&self) -> Result<Connection, ComputeControlError> {
        let connection = Connection::open(&self.database_path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Ok(connection)
    }

    pub fn reconcile(
        &self,
        platform_instances: &[CompShareInstance],
        primary_instance_id: Option<&str>,
    ) -> Result<Vec<ManagedComputeInstance>, ComputeControlError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let now = now_iso();
        for instance in platform_instances {
            upsert_platform_instance(&transaction, instance, &now)?;
        }

        let platform_ids = platform_instances
            .iter()
            .map(|instance| instance.instance_id.as_str())
            .collect::<Vec<_>>();
        let mut statement = transaction.prepare("SELECT instance_id FROM compute_instances")?;
        let known_ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        for instance_id in known_ids {
            if !platform_ids.iter().any(|current| **current == instance_id) {
                transaction.execute(
                    "UPDATE compute_instances SET
                         lifecycle_state='unknown', platform_state='Missing', running_mode='unknown',
                         missing_since=COALESCE(missing_since, ?2), updated_at=?2
                     WHERE instance_id=?1 AND lifecycle_state <> 'terminated'",
                    params![instance_id, now],
                )?;
            }
        }
        if let Some(primary_id) = primary_instance_id {
            select_primary_in_transaction(&transaction, primary_id, &now)?;
        }
        transaction.commit()?;
        self.list_instances()
    }

    pub fn adopt_primary(
        &self,
        instance: &CompShareInstance,
    ) -> Result<ManagedComputeInstance, ComputeControlError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let now = now_iso();
        upsert_platform_instance(&transaction, instance, &now)?;
        select_primary_in_transaction(&transaction, &instance.instance_id, &now)?;
        transaction.commit()?;
        self.get_instance(&instance.instance_id)
    }

    pub fn register_zhihua_instance(
        &self,
        instance: &CompShareInstance,
        role: ComputeInstanceRole,
    ) -> Result<ManagedComputeInstance, ComputeControlError> {
        if role == ComputeInstanceRole::UserManaged {
            return Err(ComputeControlError::new(
                "INVALID_COMPUTE_ROLE",
                "知画创建的实例不能登记为用户实例",
            ));
        }
        let now = now_iso();
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO compute_instances (
                instance_id, name, region, zone, project_id, role, ownership,
                cleanup_policy, lifecycle_state, platform_state, running_mode,
                gpu_type, gpu_count, image_id, release_time, stop_time,
                stop_scheduler_time, instance_price, disk_price, current_job_id,
                last_synced_at, missing_since, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, 'zhihua_managed', ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, NULL, ?19, NULL, ?19, ?19
             )
             ON CONFLICT(instance_id) DO UPDATE SET
                name=excluded.name, region=excluded.region, zone=excluded.zone,
                project_id=excluded.project_id, role=excluded.role,
                ownership='zhihua_managed', cleanup_policy=excluded.cleanup_policy,
                lifecycle_state=excluded.lifecycle_state, platform_state=excluded.platform_state,
                running_mode=excluded.running_mode, gpu_type=excluded.gpu_type,
                gpu_count=excluded.gpu_count, image_id=excluded.image_id,
                release_time=excluded.release_time, stop_time=excluded.stop_time,
                stop_scheduler_time=excluded.stop_scheduler_time,
                instance_price=excluded.instance_price, disk_price=excluded.disk_price,
                last_synced_at=excluded.last_synced_at, missing_since=NULL,
                updated_at=excluded.updated_at",
            params![
                instance.instance_id,
                instance.name,
                instance.region,
                instance.zone,
                instance.project_id,
                role.as_str(),
                if role == ComputeInstanceRole::Primary {
                    ComputeCleanupPolicy::Retain.as_str()
                } else {
                    ComputeCleanupPolicy::ReleaseWhenIdle.as_str()
                },
                lifecycle_from_platform(instance).as_str(),
                instance.raw_state,
                running_mode_text(instance.running_mode),
                instance.gpu_type,
                instance.gpu_count,
                instance.image_id,
                instance.release_time,
                instance.stop_time,
                instance.stop_scheduler_time,
                instance.instance_price,
                instance.disk_price,
                now,
            ],
        )?;
        self.get_instance(&instance.instance_id)
    }

    pub fn list_instances(&self) -> Result<Vec<ManagedComputeInstance>, ComputeControlError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT instance_id, name, region, zone, project_id, role, ownership,
                    cleanup_policy, lifecycle_state, platform_state, running_mode,
                    gpu_type, gpu_count, image_id, release_time, stop_time,
                    stop_scheduler_time, instance_price, disk_price, current_job_id,
                    last_synced_at, missing_since, updated_at
             FROM compute_instances
             ORDER BY CASE role WHEN 'primary' THEN 0 WHEN 'elastic' THEN 1
                                WHEN 'test' THEN 2 ELSE 3 END,
                      created_at ASC",
        )?;
        let rows = statement.query_map([], managed_instance_from_row)?;
        rows.map(|row| row.map_err(ComputeControlError::from))
            .collect()
    }

    pub fn get_instance(
        &self,
        instance_id: &str,
    ) -> Result<ManagedComputeInstance, ComputeControlError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT instance_id, name, region, zone, project_id, role, ownership,
                        cleanup_policy, lifecycle_state, platform_state, running_mode,
                        gpu_type, gpu_count, image_id, release_time, stop_time,
                        stop_scheduler_time, instance_price, disk_price, current_job_id,
                        last_synced_at, missing_since, updated_at
                 FROM compute_instances WHERE instance_id=?1",
                [instance_id],
                managed_instance_from_row,
            )
            .optional()?
            .ok_or_else(|| {
                ComputeControlError::new("COMPUTE_INSTANCE_NOT_FOUND", "算力实例尚未纳入管理")
            })
    }

    #[cfg(test)]
    pub fn begin_operation(
        &self,
        idempotency_key: &str,
        instance_id: Option<&str>,
        action: ComputeOperationAction,
        payload: &serde_json::Value,
    ) -> Result<ComputeOperation, ComputeControlError> {
        self.claim_operation(idempotency_key, instance_id, action, payload)
            .map(|(operation, _)| operation)
    }

    pub fn claim_operation(
        &self,
        idempotency_key: &str,
        instance_id: Option<&str>,
        action: ComputeOperationAction,
        payload: &serde_json::Value,
    ) -> Result<(ComputeOperation, bool), ComputeControlError> {
        let key = idempotency_key.trim();
        if key.is_empty() || key.chars().count() > 200 {
            return Err(ComputeControlError::new(
                "INVALID_IDEMPOTENCY_KEY",
                "算力操作幂等键不能为空且不能超过 200 个字符",
            ));
        }
        let now = now_iso();
        let id = Uuid::new_v4().to_string();
        let payload = serde_json::to_string(payload).map_err(|error| {
            ComputeControlError::new(
                "INVALID_OPERATION_PAYLOAD",
                format!("无法保存算力操作参数：{error}"),
            )
        })?;
        let connection = self.connection()?;
        let inserted = connection.execute(
            "INSERT OR IGNORE INTO compute_operations (
                id, idempotency_key, instance_id, action, status, payload_json,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6, ?6)",
            params![id, key, instance_id, action.as_str(), payload, now],
        )?;
        Ok((self.get_operation(key)?, inserted == 1))
    }

    pub fn finish_operation(
        &self,
        idempotency_key: &str,
        status: ComputeOperationStatus,
        instance_id: Option<&str>,
        request_uuid: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<ComputeOperation, ComputeControlError> {
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE compute_operations SET
                status=?2, instance_id=COALESCE(?3, instance_id), request_uuid=?4,
                error_message=?5, updated_at=?6
             WHERE idempotency_key=?1",
            params![
                idempotency_key,
                status.as_str(),
                instance_id,
                request_uuid,
                error_message,
                now_iso(),
            ],
        )?;
        if changed != 1 {
            return Err(ComputeControlError::new(
                "COMPUTE_OPERATION_NOT_FOUND",
                "找不到需要更新的算力操作记录",
            ));
        }
        self.get_operation(idempotency_key)
    }

    pub fn get_operation(
        &self,
        idempotency_key: &str,
    ) -> Result<ComputeOperation, ComputeControlError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT id, idempotency_key, instance_id, action, status,
                        request_uuid, error_message, payload_json, created_at, updated_at
                 FROM compute_operations WHERE idempotency_key=?1",
                [idempotency_key],
                operation_from_row,
            )
            .optional()?
            .ok_or_else(|| {
                ComputeControlError::new("COMPUTE_OPERATION_NOT_FOUND", "找不到算力操作记录")
            })
    }

    pub fn release_eligibility(
        &self,
        instance_id: &str,
    ) -> Result<ReleaseEligibility, ComputeControlError> {
        self.release_eligibility_excluding(instance_id, None)
    }

    pub fn release_eligibility_excluding(
        &self,
        instance_id: &str,
        excluded_idempotency_key: Option<&str>,
    ) -> Result<ReleaseEligibility, ComputeControlError> {
        let instance = self.get_instance(instance_id)?;
        let connection = self.connection()?;
        let active_leases: i64 = connection.query_row(
            "SELECT COUNT(*) FROM worker_leases
             WHERE instance_id=?1 AND lease_expires_at>?2",
            params![instance_id, now_iso()],
            |row| row.get(0),
        )?;
        let pending_operations: i64 = connection.query_row(
            "SELECT COUNT(*) FROM compute_operations
             WHERE instance_id=?1 AND status IN ('pending','unknown')
               AND (?2 IS NULL OR idempotency_key<>?2)",
            params![instance_id, excluded_idempotency_key],
            |row| row.get(0),
        )?;

        let mut reasons = Vec::new();
        if instance.ownership != ComputeInstanceOwnership::ZhihuaManaged {
            reasons.push("该实例不是由知画创建".to_owned());
        }
        if !matches!(
            instance.role,
            ComputeInstanceRole::Elastic | ComputeInstanceRole::Test
        ) {
            reasons.push("主实例或用户实例不能自动释放".to_owned());
        }
        if instance.cleanup_policy != ComputeCleanupPolicy::ReleaseWhenIdle {
            reasons.push("实例清理策略要求保留".to_owned());
        }
        if instance.platform_state.to_ascii_lowercase() != "stopped" {
            reasons.push("平台尚未确认实例已关机".to_owned());
        }
        if instance.lifecycle_state == ComputeLifecycleState::Unknown
            || instance.missing_since.is_some()
        {
            reasons.push("实例状态仍需与平台对账".to_owned());
        }
        if instance.current_job_id.is_some() || active_leases > 0 {
            reasons.push("实例仍有任务或 worker 租约".to_owned());
        }
        if pending_operations > 0 {
            reasons.push("实例仍有结果未知的平台操作".to_owned());
        }
        Ok(ReleaseEligibility {
            allowed: reasons.is_empty(),
            reasons,
        })
    }

    pub fn mark_terminating(&self, instance_id: &str) -> Result<(), ComputeControlError> {
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE compute_instances SET lifecycle_state='terminating', updated_at=?2
             WHERE instance_id=?1",
            params![instance_id, now_iso()],
        )?;
        if changed != 1 {
            return Err(ComputeControlError::new(
                "COMPUTE_INSTANCE_NOT_FOUND",
                "找不到需要释放的算力实例",
            ));
        }
        Ok(())
    }

    pub fn mark_terminated(&self, instance_id: &str) -> Result<(), ComputeControlError> {
        let connection = self.connection()?;
        connection.execute(
            "UPDATE compute_instances SET lifecycle_state='terminated', platform_state='Terminated',
                    running_mode='stopped', missing_since=NULL, updated_at=?2
             WHERE instance_id=?1",
            params![instance_id, now_iso()],
        )?;
        Ok(())
    }
}

fn upsert_platform_instance(
    transaction: &Transaction<'_>,
    instance: &CompShareInstance,
    now: &str,
) -> Result<(), ComputeControlError> {
    transaction.execute(
        "INSERT INTO compute_instances (
            instance_id, name, region, zone, project_id, role, ownership,
            cleanup_policy, lifecycle_state, platform_state, running_mode,
            gpu_type, gpu_count, image_id, release_time, stop_time,
            stop_scheduler_time, instance_price, disk_price, current_job_id,
            last_synced_at, missing_since, created_at, updated_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, 'user_managed', 'user_managed', 'retain',
            ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, NULL, ?17, NULL, ?17, ?17
         )
         ON CONFLICT(instance_id) DO UPDATE SET
            name=excluded.name, region=excluded.region, zone=excluded.zone,
            project_id=COALESCE(excluded.project_id, compute_instances.project_id),
            lifecycle_state=CASE
                WHEN compute_instances.lifecycle_state IN ('busy','draining','preparing')
                    THEN compute_instances.lifecycle_state
                ELSE excluded.lifecycle_state END,
            platform_state=excluded.platform_state, running_mode=excluded.running_mode,
            gpu_type=excluded.gpu_type, gpu_count=excluded.gpu_count,
            image_id=COALESCE(excluded.image_id, compute_instances.image_id),
            release_time=excluded.release_time, stop_time=excluded.stop_time,
            stop_scheduler_time=excluded.stop_scheduler_time,
            instance_price=excluded.instance_price, disk_price=excluded.disk_price,
            last_synced_at=excluded.last_synced_at, missing_since=NULL,
            updated_at=excluded.updated_at",
        params![
            instance.instance_id,
            instance.name,
            instance.region,
            instance.zone,
            instance.project_id,
            lifecycle_from_platform(instance).as_str(),
            instance.raw_state,
            running_mode_text(instance.running_mode),
            instance.gpu_type,
            instance.gpu_count,
            instance.image_id,
            instance.release_time,
            instance.stop_time,
            instance.stop_scheduler_time,
            instance.instance_price,
            instance.disk_price,
            now,
        ],
    )?;
    Ok(())
}

fn select_primary_in_transaction(
    transaction: &Transaction<'_>,
    instance_id: &str,
    now: &str,
) -> Result<(), ComputeControlError> {
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM compute_instances WHERE instance_id=?1)",
        [instance_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(ComputeControlError::new(
            "COMPUTE_INSTANCE_NOT_FOUND",
            "无法把平台未返回的实例设为主实例",
        ));
    }
    transaction.execute(
        "UPDATE compute_instances SET
             role=CASE ownership WHEN 'zhihua_managed' THEN 'elastic' ELSE 'user_managed' END,
             cleanup_policy=CASE ownership WHEN 'zhihua_managed' THEN 'release_when_idle' ELSE 'retain' END,
             updated_at=?1
         WHERE role='primary' AND instance_id<>?2",
        params![now, instance_id],
    )?;
    transaction.execute(
        "UPDATE compute_instances SET role='primary', cleanup_policy='retain', updated_at=?2
         WHERE instance_id=?1",
        params![instance_id, now],
    )?;
    Ok(())
}

fn lifecycle_from_platform(instance: &CompShareInstance) -> ComputeLifecycleState {
    match (instance.state, instance.running_mode) {
        (CompSharePowerState::Starting, _) => ComputeLifecycleState::Starting,
        (CompSharePowerState::Stopping, _) => ComputeLifecycleState::Stopping,
        (CompSharePowerState::Stopped, _) => ComputeLifecycleState::Retained,
        (CompSharePowerState::Running, _) => ComputeLifecycleState::Idle,
        (CompSharePowerState::Unknown, _) => ComputeLifecycleState::Unknown,
    }
}

fn running_mode_text(mode: CompShareRunningMode) -> &'static str {
    match mode {
        CompShareRunningMode::Gpu => "gpu",
        CompShareRunningMode::NoGpu => "no_gpu",
        CompShareRunningMode::Stopped => "stopped",
        CompShareRunningMode::Transitioning => "transitioning",
        CompShareRunningMode::Unknown => "unknown",
    }
}

fn managed_instance_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ManagedComputeInstance> {
    let role_text: String = row.get(5)?;
    let ownership_text: String = row.get(6)?;
    let cleanup_text: String = row.get(7)?;
    let lifecycle_text: String = row.get(8)?;
    Ok(ManagedComputeInstance {
        instance_id: row.get(0)?,
        name: row.get(1)?,
        region: row.get(2)?,
        zone: row.get(3)?,
        project_id: row.get(4)?,
        role: parse_enum(5, role_text)?,
        ownership: parse_enum(6, ownership_text)?,
        cleanup_policy: parse_enum(7, cleanup_text)?,
        lifecycle_state: parse_enum(8, lifecycle_text)?,
        platform_state: row.get(9)?,
        running_mode: row.get(10)?,
        gpu_type: row.get(11)?,
        gpu_count: row
            .get::<_, Option<i64>>(12)?
            .map(|value| u32::try_from(value).unwrap_or_default()),
        image_id: row.get(13)?,
        release_time: row.get(14)?,
        stop_time: row.get(15)?,
        stop_scheduler_time: row.get(16)?,
        instance_price: row.get(17)?,
        disk_price: row.get(18)?,
        current_job_id: row.get(19)?,
        last_synced_at: row.get(20)?,
        missing_since: row.get(21)?,
        updated_at: row.get(22)?,
    })
}

fn operation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ComputeOperation> {
    let action: String = row.get(3)?;
    let status: String = row.get(4)?;
    let payload: String = row.get(7)?;
    Ok(ComputeOperation {
        id: row.get(0)?,
        idempotency_key: row.get(1)?,
        instance_id: row.get(2)?,
        action: match action.as_str() {
            "create" => ComputeOperationAction::Create,
            "start_gpu" => ComputeOperationAction::StartGpu,
            "start_no_gpu" => ComputeOperationAction::StartNoGpu,
            "stop" => ComputeOperationAction::Stop,
            "refresh_retention" => ComputeOperationAction::RefreshRetention,
            "terminate" => ComputeOperationAction::Terminate,
            _ => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(ComputeControlError::new(
                        "INVALID_OPERATION_ACTION",
                        format!("未知的算力操作：{action}"),
                    )),
                ))
            }
        },
        status: match status.as_str() {
            "pending" => ComputeOperationStatus::Pending,
            "succeeded" => ComputeOperationStatus::Succeeded,
            "failed" => ComputeOperationStatus::Failed,
            "unknown" => ComputeOperationStatus::Unknown,
            _ => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(ComputeControlError::new(
                        "INVALID_OPERATION_STATUS",
                        format!("未知的算力操作状态：{status}"),
                    )),
                ))
            }
        },
        request_uuid: row.get(5)?,
        error_message: row.get(6)?,
        payload: serde_json::from_str(&payload).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn parse_enum<T>(index: usize, value: String) -> rusqlite::Result<T>
where
    T: FromStr,
    T::Err: Error + Send + Sync + 'static,
{
    value.parse().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store() -> ComputeControlStore {
        let temp = tempdir().unwrap();
        let path = temp.path().join("compute.sqlite3");
        let store = ComputeControlStore::initialize(path).unwrap();
        std::mem::forget(temp);
        store
    }

    fn instance(id: &str, state: CompSharePowerState) -> CompShareInstance {
        let running_mode = match state {
            CompSharePowerState::Running => CompShareRunningMode::Gpu,
            CompSharePowerState::Stopped => CompShareRunningMode::Stopped,
            CompSharePowerState::Starting | CompSharePowerState::Stopping => {
                CompShareRunningMode::Transitioning
            }
            CompSharePowerState::Unknown => CompShareRunningMode::Unknown,
        };
        CompShareInstance {
            instance_id: id.to_owned(),
            name: Some(format!("instance-{id}")),
            region: "cn-wlcb".to_owned(),
            zone: "cn-wlcb-01".to_owned(),
            state,
            raw_state: match state {
                CompSharePowerState::Running => "Running",
                CompSharePowerState::Stopped => "Stopped",
                CompSharePowerState::Starting => "Starting",
                CompSharePowerState::Stopping => "Stopping",
                CompSharePowerState::Unknown => "Unknown",
            }
            .to_owned(),
            running_mode,
            cpu: Some(16),
            memory_mb: Some(96 * 1024),
            gpu_count: Some(1),
            gpu_type: Some("5090".to_owned()),
            support_without_gpu_start: true,
            ssh_login_command: None,
            start_time: None,
            stop_time: Some(1_000),
            release_time: Some(2_000),
            stop_scheduler_time: None,
            instance_price: Some(3.15),
            disk_price: Some(0.0),
            image_price: Some(0.0),
            image_id: Some("image-1".to_owned()),
            charge_type: Some("Postpay".to_owned()),
            project_id: Some("org-1".to_owned()),
        }
    }

    #[test]
    fn discovered_instances_are_user_owned_and_retained() {
        let store = store();
        let records = store
            .reconcile(&[instance("one", CompSharePowerState::Stopped)], None)
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].role, ComputeInstanceRole::UserManaged);
        assert_eq!(records[0].ownership, ComputeInstanceOwnership::UserManaged);
        assert_eq!(records[0].cleanup_policy, ComputeCleanupPolicy::Retain);
        assert!(!store.release_eligibility("one").unwrap().allowed);
    }

    #[test]
    fn selecting_a_new_primary_demotes_the_previous_primary_safely() {
        let store = store();
        let one = instance("one", CompSharePowerState::Stopped);
        let two = instance("two", CompSharePowerState::Stopped);
        store.reconcile(&[one.clone(), two.clone()], None).unwrap();
        store.adopt_primary(&one).unwrap();
        store.adopt_primary(&two).unwrap();
        let records = store.list_instances().unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|item| item.role == ComputeInstanceRole::Primary)
                .count(),
            1
        );
        assert_eq!(
            store.get_instance("one").unwrap().role,
            ComputeInstanceRole::UserManaged
        );
        assert_eq!(
            store.get_instance("two").unwrap().role,
            ComputeInstanceRole::Primary
        );
    }

    #[test]
    fn only_stopped_owned_elastic_instances_are_releasable() {
        let store = store();
        let stopped = instance("elastic", CompSharePowerState::Stopped);
        store
            .register_zhihua_instance(&stopped, ComputeInstanceRole::Elastic)
            .unwrap();
        assert!(store.release_eligibility("elastic").unwrap().allowed);

        let running = instance("elastic", CompSharePowerState::Running);
        store.reconcile(&[running], None).unwrap();
        let eligibility = store.release_eligibility("elastic").unwrap();
        assert!(!eligibility.allowed);
        assert!(eligibility
            .reasons
            .iter()
            .any(|reason| reason.contains("关机")));
    }

    #[test]
    fn missing_platform_instances_are_kept_for_reconciliation() {
        let store = store();
        store
            .reconcile(&[instance("one", CompSharePowerState::Stopped)], None)
            .unwrap();
        store.reconcile(&[], None).unwrap();
        let record = store.get_instance("one").unwrap();
        assert_eq!(record.lifecycle_state, ComputeLifecycleState::Unknown);
        assert!(record.missing_since.is_some());
    }

    #[test]
    fn idempotent_operations_reuse_the_same_record() {
        let store = store();
        let first = store
            .begin_operation(
                "create:pool:slot-1",
                None,
                ComputeOperationAction::Create,
                &serde_json::json!({"slot": 1}),
            )
            .unwrap();
        let second = store
            .begin_operation(
                "create:pool:slot-1",
                None,
                ComputeOperationAction::Create,
                &serde_json::json!({"slot": 2}),
            )
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.payload, serde_json::json!({"slot": 1}));
    }

    #[test]
    fn unknown_terminate_operation_blocks_a_second_release() {
        let store = store();
        let stopped = instance("elastic", CompSharePowerState::Stopped);
        store
            .register_zhihua_instance(&stopped, ComputeInstanceRole::Elastic)
            .unwrap();
        store
            .begin_operation(
                "terminate:first",
                Some("elastic"),
                ComputeOperationAction::Terminate,
                &serde_json::json!({"instanceId": "elastic"}),
            )
            .unwrap();
        store
            .finish_operation(
                "terminate:first",
                ComputeOperationStatus::Unknown,
                Some("elastic"),
                None,
                Some("network result unknown"),
            )
            .unwrap();

        let eligibility = store.release_eligibility("elastic").unwrap();
        assert!(!eligibility.allowed);
        assert!(eligibility
            .reasons
            .iter()
            .any(|reason| reason.contains("结果未知")));
    }
}
