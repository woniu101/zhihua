use crate::storage::ProjectStorage;
use base64::{engine::general_purpose::STANDARD, Engine as _};
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAssetPayloadInput {
    pub project_id: String,
    pub filename: String,
    pub base64_data: String,
    pub source: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAssetInput {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceAssetFileInput {
    pub asset_id: String,
    pub source_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetCurrentAssetVersionInput {
    pub asset_id: String,
    pub version_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlinkAssetInput {
    pub asset_id: String,
    pub scene_id: Option<String>,
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
                    _links,
                )| {
                    Ok(AssetItem {
                        versions: load_versions(&connection, &id)?,
                        linked_scene_ids: linked_scenes(&connection, &project.id, &id)?,
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
            imported.push(self.import_one(
                &project.id,
                &project.project_dir,
                &value,
                "本地上传",
            )?);
        }
        Ok(imported)
    }

    pub fn import_payload(&self, input: ImportAssetPayloadInput) -> AssetResult<AssetItem> {
        if input.base64_data.len() > 28 * 1024 * 1024 {
            return Err(AssetError::new(
                "clipboard_asset_too_large",
                "剪贴板素材超过 20 MB，请改用文件导入",
            ));
        }
        let source_label = match input.source.as_str() {
            "本地上传" => "本地上传",
            "剪贴板" => "剪贴板",
            _ => return Err(AssetError::new("asset_source_invalid", "素材来源类型无效")),
        };
        let filename = Path::new(&input.filename)
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| AssetError::new("asset_name_invalid", "剪贴板素材文件名无效"))?;
        let extension = Path::new(filename)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        media_type_for(&extension)?;
        let bytes = STANDARD
            .decode(input.base64_data.trim())
            .map_err(|_| AssetError::new("clipboard_asset_invalid", "无法读取剪贴板素材"))?;
        if bytes.len() > 20 * 1024 * 1024 {
            return Err(AssetError::new(
                "clipboard_asset_too_large",
                "剪贴板素材超过 20 MB，请改用文件导入",
            ));
        }
        let project = self
            .project_storage
            .get_project(&input.project_id)
            .map_err(|error| AssetError::new("project_unavailable", error.to_string()))?;
        let temporary = project.project_dir.join("cache").join(format!(
            "clipboard-{}.{}",
            Uuid::new_v4(),
            extension
        ));
        fs::write(&temporary, bytes).map_err(file_error)?;
        let result = self.import_one(
            &project.id,
            &project.project_dir,
            &temporary.to_string_lossy(),
            source_label,
        );
        let _ = fs::remove_file(temporary);
        result
    }

    pub fn update(&self, input: UpdateAssetInput) -> AssetResult<AssetItem> {
        let name = input.name.trim();
        let description = input.description.trim();
        if name.is_empty() || name.chars().count() > 100 || description.chars().count() > 2_000 {
            return Err(AssetError::new(
                "asset_metadata_invalid",
                "素材名称或描述长度无效",
            ));
        }
        let connection = self.connection()?;
        let (project_id, media_type): (String, String) = connection
            .query_row(
                "SELECT project_id, media_type FROM assets WHERE id = ?1",
                [&input.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(database_error)?;
        validate_category(&input.category, &media_type)?;
        connection
            .execute(
                "UPDATE assets SET name = ?1, category = ?2, description = ?3, updated_at = ?4
                 WHERE id = ?5",
                params![name, input.category, description, now_iso(), input.id],
            )
            .map_err(database_error)?;
        self.find(&project_id, &input.id)
    }

    pub fn replace_file(&self, input: ReplaceAssetFileInput) -> AssetResult<AssetItem> {
        let source = PathBuf::from(input.source_path.trim());
        if !source.is_absolute() || !source.is_file() {
            return Err(AssetError::new(
                "asset_file_unavailable",
                "所选替换文件不可读取",
            ));
        }
        let mut connection = self.connection()?;
        let (project_id, media_type): (String, String) = connection
            .query_row(
                "SELECT project_id, media_type FROM assets WHERE id = ?1",
                [&input.asset_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(database_error)?;
        let project = self
            .project_storage
            .get_project(&project_id)
            .map_err(|error| AssetError::new("project_unavailable", error.to_string()))?;
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if media_type_for(&extension)? != media_type {
            return Err(AssetError::new(
                "asset_media_type_mismatch",
                "替换文件必须与原素材保持相同媒体类型",
            ));
        }
        validate_signature(&source, &extension)?;
        let metadata = fs::metadata(&source).map_err(file_error)?;
        validate_size(&media_type, metadata.len())?;
        let version_number: u32 = connection
            .query_row(
                "SELECT COALESCE(MAX(version_number), 0) + 1 FROM asset_versions WHERE asset_id = ?1",
                [&input.asset_id],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        let version_id = Uuid::new_v4().to_string();
        let destination = project
            .project_dir
            .join("assets")
            .join(media_directory(&media_type))
            .join(format!("{version_id}.{extension}"));
        copy_atomically(&source, &destination)?;
        let sha256 = sha256_path(&destination)?;
        let original_filename = source
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("替换素材")
            .to_owned();
        let timestamp = now_iso();
        let transaction = connection.transaction().map_err(database_error)?;
        if let Err(error) = transaction.execute(
            "INSERT INTO asset_versions (
                id, asset_id, version_number, original_filename, stored_path,
                format, size_bytes, sha256, note, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '替换文件，保留历史版本', ?9)",
            params![
                version_id,
                input.asset_id,
                version_number,
                original_filename,
                destination.to_string_lossy(),
                extension.to_ascii_uppercase(),
                metadata.len(),
                sha256,
                timestamp,
            ],
        ) {
            let _ = fs::remove_file(&destination);
            return Err(database_error(error));
        }
        transaction
            .execute(
                "UPDATE assets SET current_version_id = ?1, updated_at = ?2 WHERE id = ?3",
                params![version_id, timestamp, input.asset_id],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)?;
        self.find(&project_id, &input.asset_id)
    }

    pub fn set_current_version(
        &self,
        input: SetCurrentAssetVersionInput,
    ) -> AssetResult<AssetItem> {
        let connection = self.connection()?;
        let project_id: String = connection
            .query_row(
                "SELECT project_id FROM assets WHERE id = ?1",
                [&input.asset_id],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        let changed = connection
            .execute(
                "UPDATE assets SET current_version_id = ?1, updated_at = ?2
                 WHERE id = ?3 AND EXISTS(
                    SELECT 1 FROM asset_versions WHERE id = ?1 AND asset_id = ?3
                 )",
                params![input.version_id, now_iso(), input.asset_id],
            )
            .map_err(database_error)?;
        if changed != 1 {
            return Err(AssetError::new(
                "asset_version_not_found",
                "找不到要使用的素材版本",
            ));
        }
        self.find(&project_id, &input.asset_id)
    }

    pub fn delete(&self, asset_id: &str) -> AssetResult<()> {
        let connection = self.connection()?;
        let project_id: String = connection
            .query_row(
                "SELECT project_id FROM assets WHERE id = ?1",
                [asset_id],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        if !linked_scenes(&connection, &project_id, asset_id)?.is_empty() {
            return Err(AssetError::new(
                "asset_in_use",
                "素材仍被分镜使用，请先在分镜页更换参考素材",
            ));
        }
        let project = self
            .project_storage
            .get_project(&project_id)
            .map_err(|error| AssetError::new("project_unavailable", error.to_string()))?;
        let paths = load_versions(&connection, asset_id)?
            .into_iter()
            .map(|version| version.stored_path)
            .filter(|path| path.starts_with(project.project_dir.join("assets")))
            .collect::<Vec<_>>();
        connection
            .execute("DELETE FROM assets WHERE id = ?1", [asset_id])
            .map_err(database_error)?;
        for path in paths {
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    pub fn unlink(&self, input: UnlinkAssetInput) -> AssetResult<AssetItem> {
        let mut connection = self.connection()?;
        let project_id: String = connection
            .query_row(
                "SELECT project_id FROM assets WHERE id = ?1",
                [&input.asset_id],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        let links = linked_scenes(&connection, &project_id, &input.asset_id)?;
        let targets = match input.scene_id {
            Some(scene_id) if links.iter().any(|value| value == &scene_id) => vec![scene_id],
            Some(_) => {
                return Err(AssetError::new(
                    "asset_link_not_found",
                    "找不到要解除的分镜关联",
                ))
            }
            None => links,
        };
        let transaction = connection.transaction().map_err(database_error)?;
        for scene_id in targets {
            let value: String = transaction
                .query_row(
                    "SELECT asset_ids_json FROM storyboard_scenes WHERE id = ?1 AND project_id = ?2",
                    params![scene_id, project_id],
                    |row| row.get(0),
                )
                .map_err(database_error)?;
            let mut asset_ids = serde_json::from_str::<Vec<String>>(&value)
                .map_err(|_| AssetError::new("asset_record_invalid", "分镜素材关联记录已损坏"))?;
            asset_ids.retain(|id| id != &input.asset_id);
            transaction
                .execute(
                    "UPDATE storyboard_scenes SET asset_ids_json = ?1, updated_at = ?2
                     WHERE id = ?3 AND project_id = ?4",
                    params![
                        serde_json::to_string(&asset_ids).map_err(|_| AssetError::new(
                            "asset_record_invalid",
                            "无法保存分镜素材关联"
                        ))?,
                        now_iso(),
                        scene_id,
                        project_id,
                    ],
                )
                .map_err(database_error)?;
        }
        transaction.commit().map_err(database_error)?;
        self.find(&project_id, &input.asset_id)
    }

    fn find(&self, project_id: &str, asset_id: &str) -> AssetResult<AssetItem> {
        self.list(project_id)?
            .into_iter()
            .find(|asset| asset.id == asset_id)
            .ok_or_else(|| AssetError::new("asset_not_found", "找不到指定素材"))
    }

    fn import_one(
        &self,
        project_id: &str,
        project_dir: &Path,
        source_value: &str,
        source_label: &str,
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
        validate_size(media_type, metadata.len())?;
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
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', ?7, '[]', ?8, ?8)",
            params![
                asset_id,
                project_id,
                name,
                category,
                media_type,
                source_label,
                version_id,
                timestamp
            ],
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
            source: source_label.to_owned(),
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

fn linked_scenes(
    connection: &Connection,
    project_id: &str,
    asset_id: &str,
) -> AssetResult<Vec<String>> {
    let table_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'storyboard_scenes')",
            [],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    if !table_exists {
        return Ok(Vec::new());
    }
    let mut statement = connection
        .prepare("SELECT id, asset_ids_json FROM storyboard_scenes WHERE project_id = ?1")
        .map_err(database_error)?;
    let rows = statement
        .query_map([project_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(database_error)?;
    let mut linked = Vec::new();
    for row in rows {
        let (scene_id, value) = row.map_err(database_error)?;
        let asset_ids = serde_json::from_str::<Vec<String>>(&value)
            .map_err(|_| AssetError::new("asset_record_invalid", "分镜素材关联记录已损坏"))?;
        if asset_ids.iter().any(|id| id == asset_id) {
            linked.push(scene_id);
        }
    }
    Ok(linked)
}

fn validate_category(category: &str, media_type: &str) -> AssetResult<()> {
    let valid = match media_type {
        "audio" => category == "音频",
        "image" | "video" => matches!(category, "角色" | "场景" | "道具" | "风格"),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(AssetError::new(
            "asset_category_invalid",
            "素材分类与媒体类型不匹配",
        ))
    }
}

fn validate_size(media_type: &str, size: u64) -> AssetResult<()> {
    let maximum = match media_type {
        "image" => 100 * 1024 * 1024,
        "audio" => 1024 * 1024 * 1024,
        "video" => 4 * 1024 * 1024 * 1024_u64,
        _ => 0,
    };
    if size == 0 || size > maximum {
        Err(AssetError::new(
            "asset_size_invalid",
            "素材为空或超过允许大小",
        ))
    } else {
        Ok(())
    }
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
    use crate::{
        storage::CreateProjectInput,
        storyboard::{
            CandidateQuality, GenerationMode, SceneDraft, SceneStatus, StoryboardStorage,
        },
    };

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

        let updated = storage
            .update(UpdateAssetInput {
                id: imported[0].id.clone(),
                name: "闪电场景".to_owned(),
                category: "场景".to_owned(),
                description: "作为图生视频首帧".to_owned(),
            })
            .expect("update metadata");
        assert_eq!(updated.name, "闪电场景");
        assert_eq!(updated.description, "作为图生视频首帧");

        let replacement = directory.path().join("replacement.png");
        fs::write(&replacement, b"\x89PNG\r\n\x1a\nreplacement-image").expect("replacement image");
        let replaced = storage
            .replace_file(ReplaceAssetFileInput {
                asset_id: updated.id.clone(),
                source_path: replacement.to_string_lossy().into_owned(),
            })
            .expect("replace file");
        assert_eq!(replaced.versions.len(), 2);
        let first_version = replaced.versions[1].id.clone();
        let activated = storage
            .set_current_version(SetCurrentAssetVersionInput {
                asset_id: replaced.id.clone(),
                version_id: first_version.clone(),
            })
            .expect("activate older version");
        assert_eq!(activated.current_version_id, first_version);

        let pasted = storage
            .import_payload(ImportAssetPayloadInput {
                project_id: project.id.clone(),
                filename: "clipboard.png".to_owned(),
                base64_data: STANDARD.encode(b"\x89PNG\r\n\x1a\nclipboard-image"),
                source: "剪贴板".to_owned(),
            })
            .expect("clipboard import");
        assert_eq!(pasted.source, "剪贴板");
        let pasted_path = pasted.versions[0].stored_path.clone();
        storage.delete(&pasted.id).expect("delete unlinked asset");
        assert!(!pasted_path.exists());

        let storyboards = StoryboardStorage::initialize(project_storage).expect("storyboards");
        let scene_id = Uuid::new_v4().to_string();
        storyboards
            .upsert(SceneDraft {
                id: scene_id.clone(),
                project_id: project.id.clone(),
                order: 0,
                title: "引用素材的分镜".to_owned(),
                purpose: String::new(),
                source_refs: vec![],
                narration: String::new(),
                on_screen_text: vec![],
                visual_plan: "闪电".to_owned(),
                generation_mode: GenerationMode::I2v,
                target_duration_ms: 5_000,
                asset_ids: vec![activated.id.clone()],
                selected_version_id: None,
                last_job_id: None,
                last_upscale_job_id: None,
                pending_request_id: None,
                generation_stage: None,
                status: SceneStatus::Ready,
                quality: CandidateQuality::Fast,
                updated_at: now_iso(),
            })
            .expect("linked scene");
        assert_eq!(
            storage.list(&project.id).expect("linked assets")[0].linked_scene_ids,
            vec![scene_id.clone()]
        );
        assert_eq!(
            storage
                .delete(&activated.id)
                .expect_err("reject linked delete")
                .code,
            "asset_in_use"
        );
        storage
            .unlink(UnlinkAssetInput {
                asset_id: activated.id.clone(),
                scene_id: Some(scene_id),
            })
            .expect("unlink scene");
        storage.delete(&activated.id).expect("delete after unlink");
    }
}
