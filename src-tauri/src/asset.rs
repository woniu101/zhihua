use crate::storage::ProjectStorage;
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetError {
    pub code: String,
    pub message: String,
}

impl AssetError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for AssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

type AssetResult<T> = Result<T, AssetError>;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetVersion {
    pub id: String,
    pub version_number: u32,
    pub original_filename: String,
    pub stored_path: PathBuf,
    pub format: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub note: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetItem {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub category: String,
    pub media_type: String,
    pub source: String,
    pub description: String,
    pub current_version_id: String,
    pub versions: Vec<AssetVersion>,
    pub linked_scene_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAssetFilesInput {
    pub project_id: String,
    pub paths: Vec<String>,
}

#[derive(Clone)]
pub struct AssetStorage {
    database_path: PathBuf,
    project_storage: ProjectStorage,
}

impl AssetStorage {
    pub fn initialize(project_storage: ProjectStorage) -> AssetResult<Self> {
        let database_path = project_storage.info().database_path;
        let storage = Self {
            database_path,
            project_storage,
        };
        storage
            .connection()?
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS assets (
                id                 TEXT PRIMARY KEY NOT NULL,
                project_id         TEXT NOT NULL,
                name               TEXT NOT NULL,
                category           TEXT NOT NULL,
                media_type         TEXT NOT NULL,
                source             TEXT NOT NULL,
                description        TEXT NOT NULL DEFAULT '',
                current_version_id TEXT NOT NULL,
                linked_scenes_json TEXT NOT NULL DEFAULT '[]',
                created_at         TEXT NOT NULL,
                updated_at         TEXT NOT NULL,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_assets_project ON assets(project_id, updated_at DESC);
            CREATE TABLE IF NOT EXISTS asset_versions (
                id                TEXT PRIMARY KEY NOT NULL,
                asset_id          TEXT NOT NULL,
                version_number    INTEGER NOT NULL,
                original_filename TEXT NOT NULL,
                stored_path       TEXT NOT NULL UNIQUE,
                format            TEXT NOT NULL,
                size_bytes        INTEGER NOT NULL,
                sha256            TEXT NOT NULL,
                note              TEXT NOT NULL DEFAULT '',
                created_at        TEXT NOT NULL,
                FOREIGN KEY(asset_id) REFERENCES assets(id) ON DELETE CASCADE,
                UNIQUE(asset_id, version_number)
            );
            ",
            )
            .map_err(database_error)?;
        Ok(storage)
    }

    fn connection(&self) -> AssetResult<Connection> {
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

    pub fn list(&self, project_id: &str) -> AssetResult<Vec<AssetItem>> {
        let project = self
            .project_storage
            .get_project(project_id)
            .map_err(|error| AssetError::new("project_unavailable", error.to_string()))?;
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, name, category, media_type, source, description,
                        current_version_id, linked_scenes_json
                 FROM assets WHERE project_id = ?1 ORDER BY updated_at DESC, id ASC",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map([&project.id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            })
            .map_err(database_error)?;
        let records = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        records
            .into_iter()
            .map(
                |(
                    id,
                    name,
                    category,
                    media_type,
                    source,
                    description,
                    current_version_id,
                    links,
                )| {
                    Ok(AssetItem {
                        versions: load_versions(&connection, &id)?,
                        linked_scene_ids: serde_json::from_str(&links).map_err(|_| {
                            AssetError::new("asset_record_invalid", "素材关联记录已损坏")
                        })?,
                        id,
                        project_id: project.id.clone(),
                        name,
                        category,
                        media_type,
                        source,
                        description,
                        current_version_id,
                    })
                },
            )
            .collect()
    }

    pub fn import_files(&self, input: ImportAssetFilesInput) -> AssetResult<Vec<AssetItem>> {
        if input.paths.is_empty() || input.paths.len() > 50 {
            return Err(AssetError::new(
                "asset_selection_invalid",
                "每次请选择 1～50 个素材文件",
            ));
        }
        let project = self
            .project_storage
            .get_project(&input.project_id)
            .map_err(|error| AssetError::new("project_unavailable", error.to_string()))?;
        let mut imported = Vec::with_capacity(input.paths.len());
        for value in input.paths {
            imported.push(self.import_one(&project.id, &project.project_dir, &value)?);
        }
        Ok(imported)
    }

    fn import_one(
        &self,
        project_id: &str,
        project_dir: &Path,
        source_value: &str,
    ) -> AssetResult<AssetItem> {
        let source = PathBuf::from(source_value.trim());
        if !source.is_absolute() || !source.is_file() {
            return Err(AssetError::new(
                "asset_file_unavailable",
                "所选素材文件不可读取",
            ));
        }
        let original_filename = source
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| AssetError::new("asset_name_invalid", "素材文件名无效"))?
            .to_owned();
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let media_type = media_type_for(&extension)?;
        validate_signature(&source, &extension)?;
        let metadata = fs::metadata(&source).map_err(file_error)?;
        let maximum = match media_type {
            "image" => 100 * 1024 * 1024,
            "audio" => 1024 * 1024 * 1024,
            "video" => 4 * 1024 * 1024 * 1024_u64,
            _ => 0,
        };
        if metadata.len() == 0 || metadata.len() > maximum {
            return Err(AssetError::new(
                "asset_size_invalid",
                "素材为空或超过允许大小",
            ));
        }
        let asset_id = Uuid::new_v4().to_string();
        let version_id = Uuid::new_v4().to_string();
        let relative_dir = format!("assets/{}", media_directory(media_type));
        let destination_dir = project_dir.join(&relative_dir);
        fs::create_dir_all(&destination_dir).map_err(file_error)?;
        let destination = destination_dir.join(format!("{version_id}.{extension}"));
        copy_atomically(&source, &destination)?;
        let sha256 = sha256_path(&destination)?;
        let timestamp = now_iso();
        let name = source
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("未命名素材")
            .trim()
            .to_owned();
        let category = if media_type == "audio" {
            "音频"
        } else {
            "场景"
        };
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(database_error)?;
        let inserted_asset = transaction.execute(
            "INSERT INTO assets (
                id, project_id, name, category, media_type, source, description,
                current_version_id, linked_scenes_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, '本地上传', '', ?6, '[]', ?7, ?7)",
            params![asset_id, project_id, name, category, media_type, version_id, timestamp],
        );
        if let Err(error) = inserted_asset {
            let _ = fs::remove_file(&destination);
            return Err(database_error(error));
        }
        if let Err(error) = transaction.execute(
            "INSERT INTO asset_versions (
                id, asset_id, version_number, original_filename, stored_path,
                format, size_bytes, sha256, note, created_at
             ) VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, '初始版本', ?8)",
            params![
                version_id,
                asset_id,
                original_filename,
                destination.to_string_lossy(),
                extension.to_ascii_uppercase(),
                metadata.len(),
                sha256,
                timestamp
            ],
        ) {
            let _ = fs::remove_file(&destination);
            return Err(database_error(error));
        }
        transaction.commit().map_err(database_error)?;
        Ok(AssetItem {
            id: asset_id,
            project_id: project_id.to_owned(),
            name,
            category: category.to_owned(),
            media_type: media_type.to_owned(),
            source: "本地上传".to_owned(),
            description: String::new(),
            current_version_id: version_id.clone(),
            versions: vec![AssetVersion {
                id: version_id,
                version_number: 1,
                original_filename,
                stored_path: destination,
                format: extension.to_ascii_uppercase(),
                size_bytes: metadata.len(),
                sha256,
                note: "初始版本".to_owned(),
                created_at: timestamp,
            }],
            linked_scene_ids: vec![],
        })
    }
}

fn load_versions(connection: &Connection, asset_id: &str) -> AssetResult<Vec<AssetVersion>> {
    let mut statement = connection
        .prepare(
            "SELECT id, version_number, original_filename, stored_path, format,
                    size_bytes, sha256, note, created_at
             FROM asset_versions WHERE asset_id = ?1 ORDER BY version_number DESC",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map([asset_id], |row| {
            Ok(AssetVersion {
                id: row.get(0)?,
                version_number: row.get::<_, u32>(1)?,
                original_filename: row.get(2)?,
                stored_path: PathBuf::from(row.get::<_, String>(3)?),
                format: row.get(4)?,
                size_bytes: row.get(5)?,
                sha256: row.get(6)?,
                note: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .map_err(database_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(database_error)
}

fn media_type_for(extension: &str) -> AssetResult<&'static str> {
    match extension {
        "jpg" | "jpeg" | "png" | "webp" => Ok("image"),
        "mp3" | "wav" => Ok("audio"),
        "mp4" | "mov" | "webm" => Ok("video"),
        _ => Err(AssetError::new(
            "asset_type_not_supported",
            "仅支持 JPG、PNG、WebP、MP3、WAV、MP4、MOV 和 WebM",
        )),
    }
}

fn media_directory(media_type: &str) -> &'static str {
    match media_type {
        "audio" => "audio",
        "video" => "video",
        _ => "images",
    }
}

fn validate_signature(path: &Path, extension: &str) -> AssetResult<()> {
    let mut file = fs::File::open(path).map_err(file_error)?;
    let mut header = [0_u8; 16];
    let length = file.read(&mut header).map_err(file_error)?;
    let value = &header[..length];
    let valid = match extension {
        "jpg" | "jpeg" => value.starts_with(&[0xff, 0xd8, 0xff]),
        "png" => value.starts_with(b"\x89PNG\r\n\x1a\n"),
        "webp" => length >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP",
        "mp3" => {
            value.starts_with(b"ID3")
                || (length >= 2 && header[0] == 0xff && header[1] & 0xe0 == 0xe0)
        }
        "wav" => length >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WAVE",
        "mp4" | "mov" => length >= 12 && &header[4..8] == b"ftyp",
        "webm" => value.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(AssetError::new(
            "asset_content_invalid",
            "素材内容与扩展名不匹配",
        ))
    }
}

fn copy_atomically(source: &Path, destination: &Path) -> AssetResult<()> {
    let temporary = destination.with_extension("importing");
    let mut input = fs::File::open(source).map_err(file_error)?;
    let mut output = fs::File::create(&temporary).map_err(file_error)?;
    std::io::copy(&mut input, &mut output).map_err(file_error)?;
    output.flush().map_err(file_error)?;
    output.sync_all().map_err(file_error)?;
    fs::rename(&temporary, destination).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        file_error(error)
    })
}

fn sha256_path(path: &Path) -> AssetResult<String> {
    let mut file = fs::File::open(path).map_err(file_error)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let length = file.read(&mut buffer).map_err(file_error)?;
        if length == 0 {
            break;
        }
        digest.update(&buffer[..length]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn database_error(error: rusqlite::Error) -> AssetError {
    AssetError::new(
        "asset_database_error",
        format!("素材数据库操作失败：{error}"),
    )
}

fn file_error(error: std::io::Error) -> AssetError {
    AssetError::new("asset_file_error", format!("素材文件操作失败：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::CreateProjectInput;

    #[test]
    fn imports_valid_media_into_project_and_persists_metadata() {
        let directory = tempfile::tempdir().expect("temporary storage");
        let project_storage = ProjectStorage::initialize(
            directory.path().join("zhihua.sqlite3"),
            directory.path().join("projects"),
        )
        .expect("project storage");
        let project = project_storage
            .create_project(CreateProjectInput {
                title: "素材测试".to_owned(),
                audience: None,
                target_duration_sec: Some(30),
            })
            .expect("project");
        let source = directory.path().join("reference.png");
        fs::write(&source, b"\x89PNG\r\n\x1a\nvalid-test-image").expect("test image");
        let storage = AssetStorage::initialize(project_storage.clone()).expect("asset storage");
        let imported = storage
            .import_files(ImportAssetFilesInput {
                project_id: project.id.clone(),
                paths: vec![source.to_string_lossy().into_owned()],
            })
            .expect("import");
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].media_type, "image");
        assert!(imported[0].versions[0].stored_path.is_file());
        let reloaded = storage.list(&project.id).expect("reload");
        assert_eq!(
            reloaded[0].versions[0].sha256,
            imported[0].versions[0].sha256
        );
    }
}
