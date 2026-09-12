use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};
use uuid::Uuid;

const SCHEMA_VERSION: i64 = 2;

#[derive(Debug)]
pub struct StorageError(String);

impl StorageError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for StorageError {}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        Self::new(format!("文件操作失败：{error}"))
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::new(format!("本地数据库操作失败：{error}"))
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(format!("项目文件序列化失败：{error}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Draft,
    Planning,
    Generating,
    Ready,
    Completed,
}

impl ProjectStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Planning => "planning",
            Self::Generating => "generating",
            Self::Ready => "ready",
            Self::Completed => "completed",
        }
    }
}

impl FromStr for ProjectStatus {
    type Err = StorageError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "planning" => Ok(Self::Planning),
            "generating" => Ok(Self::Generating),
            "ready" => Ok(Self::Ready),
            "completed" => Ok(Self::Completed),
            _ => Err(StorageError::new(format!("未知的项目状态：{value}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub title: String,
    pub audience: Option<String>,
    pub target_duration_sec: Option<u32>,
    pub status: ProjectStatus,
    pub style_profile: serde_json::Value,
    pub project_dir: PathBuf,
    pub created_at: String,
    pub updated_at: String,
    pub last_opened_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectInput {
    pub title: String,
    pub audience: Option<String>,
    pub target_duration_sec: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProjectInput {
    pub id: String,
    pub title: Option<String>,
    pub audience: Option<String>,
    #[serde(default)]
    pub clear_audience: bool,
    pub target_duration_sec: Option<u32>,
    #[serde(default)]
    pub clear_target_duration: bool,
    pub status: Option<ProjectStatus>,
    pub style_profile: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    pub database_path: PathBuf,
    pub projects_root: PathBuf,
    pub schema_version: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    pub project_bytes: u64,
    pub database_bytes: u64,
    pub available_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct ProjectStorage {
    database_path: PathBuf,
    projects_root: PathBuf,
}

impl ProjectStorage {
    pub fn initialize(
        database_path: impl Into<PathBuf>,
        projects_root: impl Into<PathBuf>,
    ) -> Result<Self, StorageError> {
        let storage = Self {
            database_path: database_path.into(),
            projects_root: projects_root.into(),
        };

        if let Some(parent) = storage.database_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir_all(&storage.projects_root)?;

        let connection = storage.connection()?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS projects (
                id                  TEXT PRIMARY KEY NOT NULL,
                title               TEXT NOT NULL,
                audience            TEXT,
                target_duration_sec INTEGER,
                status              TEXT NOT NULL,
                style_profile_json  TEXT NOT NULL DEFAULT '{}',
                project_dir         TEXT NOT NULL UNIQUE,
                created_at          TEXT NOT NULL,
                updated_at          TEXT NOT NULL,
                last_opened_at      TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_projects_recent
                ON projects(last_opened_at DESC, updated_at DESC);
            ",
        )?;
        connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(storage)
    }

    pub fn info(&self) -> StorageInfo {
        StorageInfo {
            database_path: self.database_path.clone(),
            projects_root: self.projects_root.clone(),
            schema_version: SCHEMA_VERSION,
        }
    }

    pub fn usage(&self) -> Result<StorageUsage, StorageError> {
        Ok(StorageUsage {
            project_bytes: directory_size(&self.projects_root)?,
            database_bytes: fs::metadata(&self.database_path)
                .map(|value| value.len())
                .unwrap_or(0),
            available_bytes: fs2::available_space(&self.projects_root)?,
            total_bytes: fs2::total_space(&self.projects_root)?,
        })
    }

    fn connection(&self) -> Result<Connection, StorageError> {
        let connection = Connection::open(&self.database_path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Ok(connection)
    }

    pub fn create_project(&self, input: CreateProjectInput) -> Result<Project, StorageError> {
        let title = validate_title(input.title)?;
        validate_duration(input.target_duration_sec)?;
        let id = Uuid::new_v4().to_string();
        let project_dir = self.projects_root.join(&id);
        let now = now_iso();
        let project = Project {
            id,
            title,
            audience: normalize_optional_text(input.audience),
            target_duration_sec: input.target_duration_sec,
            status: ProjectStatus::Draft,
            style_profile: serde_json::json!({}),
            project_dir,
            created_at: now.clone(),
            updated_at: now,
            last_opened_at: None,
        };

        create_project_directories(&project.project_dir)?;
        if let Err(error) = write_project_manifest(&project) {
            let _ = fs::remove_dir_all(&project.project_dir);
            return Err(error);
        }
        if let Err(error) = self.insert_project(&project) {
            let _ = fs::remove_dir_all(&project.project_dir);
            return Err(error);
        }
        Ok(project)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>, StorageError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, title, audience, target_duration_sec, status,
                    style_profile_json, project_dir, created_at, updated_at, last_opened_at
             FROM projects
             ORDER BY COALESCE(last_opened_at, updated_at) DESC, updated_at DESC, id ASC",
        )?;
        let rows = statement.query_map([], project_from_row)?;
        rows.map(|row| row.map_err(StorageError::from)).collect()
    }

    pub fn get_project(&self, id: &str) -> Result<Project, StorageError> {
        validate_id(id)?;
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT id, title, audience, target_duration_sec, status,
                        style_profile_json, project_dir, created_at, updated_at, last_opened_at
                 FROM projects WHERE id = ?1",
                [id],
                project_from_row,
            )
            .optional()?
            .ok_or_else(|| StorageError::new("项目不存在或已被删除"))
    }

    pub fn update_project(&self, input: UpdateProjectInput) -> Result<Project, StorageError> {
        let mut project = self.get_project(&input.id)?;
        let previous = project.clone();

        if let Some(title) = input.title {
            project.title = validate_title(title)?;
        }
        if input.clear_audience {
            project.audience = None;
        } else if input.audience.is_some() {
            project.audience = normalize_optional_text(input.audience);
        }
        if input.clear_target_duration {
            project.target_duration_sec = None;
        } else if let Some(duration) = input.target_duration_sec {
            validate_duration(Some(duration))?;
            project.target_duration_sec = Some(duration);
        }
        if let Some(status) = input.status {
            project.status = status;
        }
        if let Some(style_profile) = input.style_profile {
            if !style_profile.is_object() {
                return Err(StorageError::new("项目视觉规范必须是 JSON 对象"));
            }
            project.style_profile = style_profile;
        }
        project.updated_at = next_timestamp(&previous.updated_at);
        self.persist_changed_project(&previous, &project)?;
        Ok(project)
    }

    pub fn rename_project(&self, id: &str, title: String) -> Result<Project, StorageError> {
        self.update_project(UpdateProjectInput {
            id: id.to_owned(),
            title: Some(title),
            audience: None,
            clear_audience: false,
            target_duration_sec: None,
            clear_target_duration: false,
            status: None,
            style_profile: None,
        })
    }

    pub fn mark_project_opened(&self, id: &str) -> Result<Project, StorageError> {
        let mut project = self.get_project(id)?;
        let previous = project.clone();
        project.last_opened_at = Some(match previous.last_opened_at.as_deref() {
            Some(timestamp) => next_timestamp(timestamp),
            None => now_iso(),
        });
        self.persist_changed_project(&previous, &project)?;
        Ok(project)
    }

    pub fn duplicate_project(
        &self,
        id: &str,
        title: Option<String>,
    ) -> Result<Project, StorageError> {
        let source = self.get_project(id)?;
        let copied_title = match title {
            Some(title) => validate_title(title)?,
            None => format!("{} - 副本", source.title),
        };
        let new_id = Uuid::new_v4().to_string();
        let new_dir = self.projects_root.join(&new_id);
        if let Err(error) = copy_directory(&source.project_dir, &new_dir) {
            let _ = fs::remove_dir_all(&new_dir);
            return Err(error);
        }

        let now = now_iso();
        let copy = Project {
            id: new_id,
            title: copied_title,
            audience: source.audience,
            target_duration_sec: source.target_duration_sec,
            status: ProjectStatus::Draft,
            style_profile: source.style_profile,
            project_dir: new_dir,
            created_at: now.clone(),
            updated_at: now,
            last_opened_at: None,
        };

        if let Err(error) = write_project_manifest(&copy) {
            let _ = fs::remove_dir_all(&copy.project_dir);
            return Err(error);
        }
        if let Err(error) = self.insert_project(&copy) {
            let _ = fs::remove_dir_all(&copy.project_dir);
            return Err(error);
        }
        Ok(copy)
    }

    pub fn delete_project(&self, id: &str) -> Result<(), StorageError> {
        let project = self.get_project(id)?;
        let trash_dir = self.projects_root.join(format!(".deleting-{}", project.id));
        if trash_dir.exists() {
            fs::remove_dir_all(&trash_dir)?;
        }
        fs::rename(&project.project_dir, &trash_dir)?;

        let result = (|| -> Result<(), StorageError> {
            let connection = self.connection()?;
            let changed = connection.execute("DELETE FROM projects WHERE id = ?1", [id])?;
            if changed != 1 {
                return Err(StorageError::new("项目不存在或已被删除"));
            }
            Ok(())
        })();

        if let Err(error) = result {
            let _ = fs::rename(&trash_dir, &project.project_dir);
            return Err(error);
        }
        fs::remove_dir_all(trash_dir)?;
        Ok(())
    }

    fn insert_project(&self, project: &Project) -> Result<(), StorageError> {
        let connection = self.connection()?;
        let style_profile = serde_json::to_string(&project.style_profile)?;
        connection.execute(
            "INSERT INTO projects (
                id, title, audience, target_duration_sec, status, style_profile_json,
                project_dir, created_at, updated_at, last_opened_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                project.id,
                project.title,
                project.audience,
                project.target_duration_sec,
                project.status.as_str(),
                style_profile,
                path_to_string(&project.project_dir),
                project.created_at,
                project.updated_at,
                project.last_opened_at,
            ],
        )?;
        Ok(())
    }

    fn persist_changed_project(
        &self,
        previous: &Project,
        project: &Project,
    ) -> Result<(), StorageError> {
        write_project_manifest(project)?;
        let result = (|| -> Result<(), StorageError> {
            let connection = self.connection()?;
            let style_profile = serde_json::to_string(&project.style_profile)?;
            let changed = connection.execute(
                "UPDATE projects SET
                    title = ?2, audience = ?3, target_duration_sec = ?4, status = ?5,
                    style_profile_json = ?6, updated_at = ?7, last_opened_at = ?8
                 WHERE id = ?1",
                params![
                    project.id,
                    project.title,
                    project.audience,
                    project.target_duration_sec,
                    project.status.as_str(),
                    style_profile,
                    project.updated_at,
                    project.last_opened_at,
                ],
            )?;
            if changed != 1 {
                return Err(StorageError::new("项目不存在或已被删除"));
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = write_project_manifest(previous);
            return Err(error);
        }
        Ok(())
    }
}

fn directory_size(path: &Path) -> Result<u64, StorageError> {
    let mut total = 0_u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total = total.saturating_add(directory_size(&entry.path())?);
        } else if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    let status_text: String = row.get(4)?;
    let style_profile_text: String = row.get(5)?;
    let target_duration: Option<i64> = row.get(3)?;
    let status = ProjectStatus::from_str(&status_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let style_profile = serde_json::from_str(&style_profile_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let target_duration_sec = target_duration
        .map(|duration| {
            u32::try_from(duration).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Integer,
                    Box::new(error),
                )
            })
        })
        .transpose()?;

    Ok(Project {
        id: row.get(0)?,
        title: row.get(1)?,
        audience: row.get(2)?,
        target_duration_sec,
        status,
        style_profile,
        project_dir: PathBuf::from(row.get::<_, String>(6)?),
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        last_opened_at: row.get(9)?,
    })
}

fn validate_title(title: String) -> Result<String, StorageError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(StorageError::new("项目名称不能为空"));
    }
    if title.chars().count() > 100 {
        return Err(StorageError::new("项目名称不能超过 100 个字符"));
    }
    Ok(title.to_owned())
}

fn validate_id(id: &str) -> Result<(), StorageError> {
    Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| StorageError::new("项目 ID 格式无效"))
}

fn validate_duration(duration: Option<u32>) -> Result<(), StorageError> {
    if duration == Some(0) {
        return Err(StorageError::new("目标时长必须大于 0 秒"));
    }
    Ok(())
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_owned())
    })
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn next_timestamp(previous: &str) -> String {
    let current = now_iso();
    if current.as_str() > previous {
        current
    } else {
        chrono::DateTime::parse_from_rfc3339(previous)
            .map(|time| {
                (time + chrono::Duration::milliseconds(1))
                    .to_rfc3339_opts(SecondsFormat::Millis, true)
            })
            .unwrap_or(current)
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn create_project_directories(project_dir: &Path) -> Result<(), StorageError> {
    for relative in [
        "sources",
        "extracted",
        "assets/images",
        "assets/audio",
        "assets/video",
        "generated/images",
        "generated/videos",
        "generated/audio",
        "scenes",
        "subtitles",
        "exports/clips",
        "exports/packages",
        "exports/final",
        "cache",
    ] {
        fs::create_dir_all(project_dir.join(relative))?;
    }
    Ok(())
}

fn write_project_manifest(project: &Project) -> Result<(), StorageError> {
    let contents = serde_json::to_vec_pretty(project)?;
    let path = project.project_dir.join("project.json");
    fs::write(path, contents)?;
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), StorageError> {
    if !source.is_dir() {
        return Err(StorageError::new("项目目录不存在，无法复制"));
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_directory(&entry.path(), &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), destination_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage() -> (tempfile::TempDir, ProjectStorage) {
        let directory = tempfile::tempdir().expect("temporary directory");
        let storage = ProjectStorage::initialize(
            directory.path().join("zhihua.sqlite3"),
            directory.path().join("projects"),
        )
        .expect("initialize storage");
        (directory, storage)
    }

    fn create(storage: &ProjectStorage, title: &str) -> Project {
        storage
            .create_project(CreateProjectInput {
                title: title.to_owned(),
                audience: Some("初中生".to_owned()),
                target_duration_sec: Some(90),
            })
            .expect("create project")
    }

    #[test]
    fn creates_database_manifest_and_expected_directory_layout() {
        let (_directory, storage) = storage();
        let project = create(&storage, "为什么会打雷？");

        assert_eq!(project.status, ProjectStatus::Draft);
        assert!(project.project_dir.join("project.json").is_file());
        for relative in [
            "sources",
            "extracted",
            "assets/images",
            "assets/audio",
            "generated/images",
            "generated/videos",
            "generated/audio",
            "scenes",
            "subtitles",
            "exports/clips",
            "exports/packages",
            "exports/final",
            "cache",
        ] {
            assert!(project.project_dir.join(relative).is_dir(), "{relative}");
        }
        assert_eq!(
            storage.list_projects().expect("list projects"),
            vec![project]
        );
    }

    #[test]
    fn updates_renames_and_records_recent_open_time() {
        let (_directory, storage) = storage();
        let project = create(&storage, "旧名称");
        let renamed = storage
            .rename_project(&project.id, " 新名称 ".to_owned())
            .expect("rename project");
        assert_eq!(renamed.title, "新名称");
        assert!(renamed.updated_at > project.updated_at);

        let updated = storage
            .update_project(UpdateProjectInput {
                id: project.id.clone(),
                title: None,
                audience: None,
                clear_audience: true,
                target_duration_sec: Some(120),
                clear_target_duration: false,
                status: Some(ProjectStatus::Planning),
                style_profile: Some(serde_json::json!({"aspectRatio": "16:9"})),
            })
            .expect("update project");
        assert_eq!(updated.audience, None);
        assert_eq!(updated.target_duration_sec, Some(120));
        assert_eq!(updated.status, ProjectStatus::Planning);

        let opened = storage
            .mark_project_opened(&project.id)
            .expect("mark opened");
        assert!(opened.last_opened_at.is_some());
        assert_eq!(storage.list_projects().expect("list projects")[0], opened);
    }

    #[test]
    fn duplicates_all_project_files_and_deletes_only_requested_project() {
        let (_directory, storage) = storage();
        let project = create(&storage, "原项目");
        fs::write(project.project_dir.join("sources/source.txt"), "source").expect("write source");

        let duplicate = storage
            .duplicate_project(&project.id, None)
            .expect("duplicate project");
        assert_eq!(duplicate.title, "原项目 - 副本");
        assert_ne!(duplicate.id, project.id);
        assert_eq!(
            fs::read_to_string(duplicate.project_dir.join("sources/source.txt"))
                .expect("read copied source"),
            "source"
        );

        storage
            .delete_project(&project.id)
            .expect("delete original project");
        assert!(!project.project_dir.exists());
        assert!(duplicate.project_dir.exists());
        assert_eq!(
            storage.list_projects().expect("list projects"),
            vec![duplicate]
        );
    }

    #[test]
    fn rejects_invalid_project_values_without_leaving_directories() {
        let (_directory, storage) = storage();
        let error = storage
            .create_project(CreateProjectInput {
                title: "   ".to_owned(),
                audience: None,
                target_duration_sec: None,
            })
            .expect_err("empty title should fail");
        assert!(error.to_string().contains("不能为空"));
        assert!(fs::read_dir(&storage.projects_root)
            .expect("read projects root")
            .next()
            .is_none());
    }
}
