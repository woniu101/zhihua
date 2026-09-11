use crate::{frame_profile::FrameAspectRatio, storage::ProjectStorage};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf, time::Duration};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameCompositionError {
    pub code: String,
    pub message: String,
}

impl FrameCompositionError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for FrameCompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

type CompositionResult<T> = Result<T, FrameCompositionError>;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameComposition {
    pub id: String,
    pub project_id: String,
    pub asset_id: String,
    pub asset_version_id: String,
    pub aspect_ratio: String,
    pub fit_mode: String,
    pub focal_x: f64,
    pub focal_y: f64,
    pub background_mode: String,
    pub work_width: u32,
    pub work_height: u32,
    pub visible_width: u32,
    pub visible_height: u32,
    pub crop_x: u32,
    pub crop_y: u32,
    pub derivative_path: Option<PathBuf>,
    pub derivative_sha256: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFrameCompositionInput {
    pub project_id: String,
    pub asset_id: String,
    pub asset_version_id: String,
    pub aspect_ratio: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFrameCompositionInput {
    pub project_id: String,
    pub asset_id: String,
    pub asset_version_id: String,
    pub aspect_ratio: String,
    pub fit_mode: String,
    pub focal_x: f64,
    pub focal_y: f64,
    pub background_mode: String,
}

#[derive(Clone)]
pub struct FrameCompositionStorage {
    database_path: PathBuf,
}

impl FrameCompositionStorage {
    pub fn initialize(projects: ProjectStorage) -> CompositionResult<Self> {
        let storage = Self {
            database_path: projects.info().database_path,
        };
        storage
            .connection()?
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS frame_compositions (
                    id                  TEXT PRIMARY KEY NOT NULL,
                    project_id          TEXT NOT NULL,
                    asset_id            TEXT NOT NULL,
                    asset_version_id    TEXT NOT NULL,
                    aspect_ratio        TEXT NOT NULL,
                    fit_mode            TEXT NOT NULL,
                    focal_x             REAL NOT NULL,
                    focal_y             REAL NOT NULL,
                    background_mode     TEXT NOT NULL,
                    work_width          INTEGER NOT NULL,
                    work_height         INTEGER NOT NULL,
                    visible_width       INTEGER NOT NULL,
                    visible_height      INTEGER NOT NULL,
                    crop_x              INTEGER NOT NULL,
                    crop_y              INTEGER NOT NULL,
                    derivative_path     TEXT,
                    derivative_sha256   TEXT,
                    updated_at          TEXT NOT NULL,
                    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE,
                    FOREIGN KEY(asset_id) REFERENCES assets(id) ON DELETE CASCADE,
                    FOREIGN KEY(asset_version_id) REFERENCES asset_versions(id) ON DELETE CASCADE,
                    UNIQUE(asset_version_id, aspect_ratio)
                );
                CREATE INDEX IF NOT EXISTS idx_frame_compositions_project
                    ON frame_compositions(project_id, asset_id, aspect_ratio);
                ",
            )
            .map_err(database_error)?;
        Ok(storage)
    }

    fn connection(&self) -> CompositionResult<Connection> {
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

    pub fn get(
        &self,
        input: GetFrameCompositionInput,
    ) -> CompositionResult<Option<FrameComposition>> {
        let aspect_ratio = validate_aspect(&input.aspect_ratio)?
            .profile()
            .aspect_ratio
            .label();
        self.connection()?
            .query_row(
                "SELECT id, project_id, asset_id, asset_version_id, aspect_ratio,
                        fit_mode, focal_x, focal_y, background_mode,
                        work_width, work_height, visible_width, visible_height,
                        crop_x, crop_y, derivative_path, derivative_sha256, updated_at
                 FROM frame_compositions
                 WHERE project_id=?1 AND asset_id=?2 AND asset_version_id=?3 AND aspect_ratio=?4",
                params![
                    input.project_id,
                    input.asset_id,
                    input.asset_version_id,
                    aspect_ratio
                ],
                composition_from_row,
            )
            .optional()
            .map_err(database_error)
    }

    pub fn save(&self, input: SaveFrameCompositionInput) -> CompositionResult<FrameComposition> {
        let profile = validate_aspect(&input.aspect_ratio)?.profile();
        let aspect_ratio = profile.aspect_ratio.label();
        if !matches!(input.fit_mode.as_str(), "cover" | "contain") {
            return Err(FrameCompositionError::new(
                "frame_fit_invalid",
                "画幅适配方式必须为裁切填满或完整显示。",
            ));
        }
        if !matches!(input.background_mode.as_str(), "edge" | "blur" | "solid") {
            return Err(FrameCompositionError::new(
                "frame_background_invalid",
                "画幅留白方式无效。",
            ));
        }
        if !input.focal_x.is_finite()
            || !input.focal_y.is_finite()
            || !(0.0..=1.0).contains(&input.focal_x)
            || !(0.0..=1.0).contains(&input.focal_y)
        {
            return Err(FrameCompositionError::new(
                "frame_focus_invalid",
                "主体位置必须位于画面范围内。",
            ));
        }
        let connection = self.connection()?;
        let asset: Option<(String, String)> = connection
            .query_row(
                "SELECT a.project_id, a.media_type
                 FROM asset_versions v JOIN assets a ON a.id=v.asset_id
                 WHERE a.id=?1 AND v.id=?2",
                params![input.asset_id, input.asset_version_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(database_error)?;
        let Some((project_id, media_type)) = asset else {
            return Err(FrameCompositionError::new(
                "frame_asset_not_found",
                "找不到要适配的图片版本。",
            ));
        };
        if project_id != input.project_id || media_type != "image" {
            return Err(FrameCompositionError::new(
                "frame_asset_invalid",
                "画幅适配只适用于当前项目的图片版本。",
            ));
        }

        let id = connection
            .query_row(
                "SELECT id FROM frame_compositions WHERE asset_version_id=?1 AND aspect_ratio=?2",
                params![input.asset_version_id, aspect_ratio],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error)?
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let updated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        connection
            .execute(
                "INSERT INTO frame_compositions (
                    id, project_id, asset_id, asset_version_id, aspect_ratio,
                    fit_mode, focal_x, focal_y, background_mode,
                    work_width, work_height, visible_width, visible_height,
                    crop_x, crop_y, updated_at
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
                 ON CONFLICT(asset_version_id, aspect_ratio) DO UPDATE SET
                    fit_mode=excluded.fit_mode,
                    focal_x=excluded.focal_x,
                    focal_y=excluded.focal_y,
                    background_mode=excluded.background_mode,
                    work_width=excluded.work_width,
                    work_height=excluded.work_height,
                    visible_width=excluded.visible_width,
                    visible_height=excluded.visible_height,
                    crop_x=excluded.crop_x,
                    crop_y=excluded.crop_y,
                    derivative_path=NULL,
                    derivative_sha256=NULL,
                    updated_at=excluded.updated_at",
                params![
                    id,
                    input.project_id,
                    input.asset_id,
                    input.asset_version_id,
                    aspect_ratio,
                    input.fit_mode,
                    input.focal_x,
                    input.focal_y,
                    input.background_mode,
                    profile.work.width,
                    profile.work.height,
                    profile.visible.width,
                    profile.visible.height,
                    profile.crop_x,
                    profile.crop_y,
                    updated_at,
                ],
            )
            .map_err(database_error)?;
        self.get(GetFrameCompositionInput {
            project_id,
            asset_id: input.asset_id,
            asset_version_id: input.asset_version_id,
            aspect_ratio: aspect_ratio.to_owned(),
        })?
        .ok_or_else(|| FrameCompositionError::new("frame_save_failed", "画幅适配保存后无法读取。"))
    }
}

fn validate_aspect(value: &str) -> CompositionResult<FrameAspectRatio> {
    FrameAspectRatio::from_label(value)
        .ok_or_else(|| FrameCompositionError::new("frame_aspect_invalid", "项目画幅不受支持。"))
}

fn composition_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FrameComposition> {
    Ok(FrameComposition {
        id: row.get(0)?,
        project_id: row.get(1)?,
        asset_id: row.get(2)?,
        asset_version_id: row.get(3)?,
        aspect_ratio: row.get(4)?,
        fit_mode: row.get(5)?,
        focal_x: row.get(6)?,
        focal_y: row.get(7)?,
        background_mode: row.get(8)?,
        work_width: row.get(9)?,
        work_height: row.get(10)?,
        visible_width: row.get(11)?,
        visible_height: row.get(12)?,
        crop_x: row.get(13)?,
        crop_y: row.get(14)?,
        derivative_path: row.get::<_, Option<String>>(15)?.map(PathBuf::from),
        derivative_sha256: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

fn database_error(error: rusqlite::Error) -> FrameCompositionError {
    FrameCompositionError::new(
        "frame_database_error",
        format!("画幅适配数据库操作失败：{error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        asset::{AssetStorage, ImportAssetFilesInput},
        storage::CreateProjectInput,
    };
    use std::fs;

    #[test]
    fn keeps_an_independent_non_destructive_composition_for_each_frame() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let projects = ProjectStorage::initialize(
            directory.path().join("db.sqlite3"),
            directory.path().join("projects"),
        )
        .expect("project storage");
        let project = projects
            .create_project(CreateProjectInput {
                title: "画幅测试".to_owned(),
                audience: None,
                target_duration_sec: Some(30),
            })
            .expect("project");
        let source = directory.path().join("source.png");
        fs::write(&source, b"\x89PNG\r\n\x1a\nframe-composition-test").expect("source image");
        let assets = AssetStorage::initialize(projects.clone()).expect("asset storage");
        let asset = assets
            .import_files(ImportAssetFilesInput {
                project_id: project.id.clone(),
                paths: vec![source.to_string_lossy().into_owned()],
            })
            .expect("import image")
            .remove(0);
        let version_id = asset.current_version_id.clone();
        let storage = FrameCompositionStorage::initialize(projects).expect("compositions");
        let saved = storage
            .save(SaveFrameCompositionInput {
                project_id: project.id.clone(),
                asset_id: asset.id.clone(),
                asset_version_id: version_id.clone(),
                aspect_ratio: "9:16".to_owned(),
                fit_mode: "cover".to_owned(),
                focal_x: 0.35,
                focal_y: 0.65,
                background_mode: "edge".to_owned(),
            })
            .expect("save composition");
        assert_eq!(saved.visible_width, 756);
        assert_eq!(saved.visible_height, 1344);
        assert_eq!(saved.crop_x, 6);
        assert_eq!(saved.focal_x, 0.35);

        let landscape = storage
            .get(GetFrameCompositionInput {
                project_id: project.id,
                asset_id: asset.id,
                asset_version_id: version_id,
                aspect_ratio: "16:9".to_owned(),
            })
            .expect("read other frame");
        assert!(landscape.is_none());
    }
}
