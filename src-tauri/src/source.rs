use crate::storage::ProjectStorage;
use chrono::{SecondsFormat, Utc};
use encoding_rs::{GB18030, UTF_16BE, UTF_16LE};
use quick_xml::{events::Event, Reader};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fmt, fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};
use uuid::Uuid;
use zip::ZipArchive;

const MAX_SOURCE_BYTES: u64 = 200 * 1024 * 1024;
const MAX_XML_BYTES: u64 = 32 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: usize = 20 * 1024 * 1024;
const MAX_PASTED_BYTES: usize = 5 * 1024 * 1024;
const MAX_ZIP_ENTRIES: usize = 4_096;

#[derive(Debug)]
pub struct SourceError(String);

impl SourceError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SourceError {}

impl From<std::io::Error> for SourceError {
    fn from(error: std::io::Error) -> Self {
        Self::new(format!("文件操作失败：{error}"))
    }
}

impl From<rusqlite::Error> for SourceError {
    fn from(error: rusqlite::Error) -> Self {
        Self::new(format!("资料数据库操作失败：{error}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    Txt,
    Pdf,
    Docx,
    Pptx,
    Image,
    PastedText,
}

impl SourceType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Txt => "txt",
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Pptx => "pptx",
            Self::Image => "image",
            Self::PastedText => "pasted_text",
        }
    }
}

impl FromStr for SourceType {
    type Err = SourceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "txt" => Ok(Self::Txt),
            "pdf" => Ok(Self::Pdf),
            "docx" => Ok(Self::Docx),
            "pptx" => Ok(Self::Pptx),
            "image" => Ok(Self::Image),
            "pasted_text" => Ok(Self::PastedText),
            _ => Err(SourceError::new(format!("未知的资料类型：{value}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceParseStatus {
    Ready,
    NoText,
    Failed,
}

impl SourceParseStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NoText => "no_text",
            Self::Failed => "failed",
        }
    }
}

impl FromStr for SourceParseStatus {
    type Err = SourceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ready" => Ok(Self::Ready),
            "no_text" => Ok(Self::NoText),
            "failed" => Ok(Self::Failed),
            _ => Err(SourceError::new(format!("未知的资料解析状态：{value}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub source_type: SourceType,
    pub original_file_name: Option<String>,
    pub original_path: Option<PathBuf>,
    pub stored_path: PathBuf,
    pub extracted_path: Option<PathBuf>,
    pub byte_size: u64,
    pub extracted_char_count: u64,
    pub enabled: bool,
    pub parse_status: SourceParseStatus,
    pub parse_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSourceFileInput {
    pub project_id: String,
    pub file_path: PathBuf,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePastedSourceInput {
    pub project_id: String,
    pub name: Option<String>,
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSourceEnabledInput {
    pub id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct SourceStorage {
    project_storage: ProjectStorage,
    database_path: PathBuf,
}

impl SourceStorage {
    pub fn initialize(project_storage: ProjectStorage) -> Result<Self, SourceError> {
        let database_path = project_storage.info().database_path;
        let storage = Self {
            project_storage,
            database_path,
        };
        let connection = storage.connection()?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sources (
                id                   TEXT PRIMARY KEY NOT NULL,
                project_id           TEXT NOT NULL,
                name                 TEXT NOT NULL,
                source_type          TEXT NOT NULL,
                original_file_name   TEXT,
                original_path        TEXT,
                stored_path          TEXT NOT NULL UNIQUE,
                extracted_path       TEXT,
                byte_size            INTEGER NOT NULL,
                extracted_char_count INTEGER NOT NULL DEFAULT 0,
                enabled              INTEGER NOT NULL DEFAULT 1,
                parse_status         TEXT NOT NULL,
                parse_message        TEXT,
                created_at           TEXT NOT NULL,
                updated_at           TEXT NOT NULL,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_sources_project_created
                ON sources(project_id, created_at ASC, id ASC);
            ",
        )?;
        Ok(storage)
    }

    fn connection(&self) -> Result<Connection, SourceError> {
        let connection = Connection::open(&self.database_path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Ok(connection)
    }

    pub fn list_sources(&self, project_id: &str) -> Result<Vec<Source>, SourceError> {
        self.project_storage
            .get_project(project_id)
            .map_err(|error| SourceError::new(error.to_string()))?;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, project_id, name, source_type, original_file_name, original_path,
                    stored_path, extracted_path, byte_size, extracted_char_count, enabled,
                    parse_status, parse_message, created_at, updated_at
             FROM sources WHERE project_id = ?1
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = statement.query_map([project_id], source_from_row)?;
        rows.map(|row| row.map_err(SourceError::from)).collect()
    }

    pub fn import_file(&self, input: ImportSourceFileInput) -> Result<Source, SourceError> {
        let project = self
            .project_storage
            .get_project(&input.project_id)
            .map_err(|error| SourceError::new(error.to_string()))?;
        let roots = ProjectSourceRoots::resolve(
            &self.project_storage.info().projects_root,
            &project.project_dir,
        )?;
        let input_path = fs::canonicalize(&input.file_path).map_err(|error| {
            SourceError::new(format!(
                "无法读取所选资料文件 {}：{error}",
                input.file_path.display()
            ))
        })?;
        if !input_path.is_file() {
            return Err(SourceError::new("所选路径不是文件"));
        }
        let metadata = fs::metadata(&input_path)?;
        if metadata.len() > MAX_SOURCE_BYTES {
            return Err(SourceError::new("资料文件超过 200 MB，MVP 暂不支持导入"));
        }

        let original_file_name = input_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| SourceError::new("资料文件名不是有效的 Unicode 文本"))?
            .to_owned();
        let extension = input_path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| SourceError::new("资料文件缺少扩展名"))?;
        let source_type = source_type_for_extension(&extension)?;
        validate_file_signature(&input_path, source_type, &extension)?;
        let name = validate_source_name(input.name.as_deref().unwrap_or(&original_file_name))?;
        let id = Uuid::new_v4().to_string();
        let stored_path = roots.sources.join(format!("{id}.{extension}"));
        ensure_child_path(&stored_path, &roots.sources)?;
        fs::copy(&input_path, &stored_path)
            .map_err(|error| SourceError::new(format!("复制资料文件失败：{error}")))?;

        let extraction = extract_source(&stored_path, source_type);
        let (extracted_path, extracted_char_count, parse_status, parse_message) = match extraction {
            Ok(ExtractedContent::Text(text)) => {
                let extracted_path = roots.extracted.join(format!("{id}.txt"));
                ensure_child_path(&extracted_path, &roots.extracted)?;
                if let Err(error) = fs::write(&extracted_path, text.as_bytes()) {
                    let _ = fs::remove_file(&stored_path);
                    return Err(SourceError::new(format!("保存提取文本失败：{error}")));
                }
                (
                    Some(extracted_path),
                    text.chars().count() as u64,
                    SourceParseStatus::Ready,
                    None,
                )
            }
            Ok(ExtractedContent::NoText(message)) => {
                (None, 0, SourceParseStatus::NoText, Some(message))
            }
            Err(error) => (None, 0, SourceParseStatus::Failed, Some(error.to_string())),
        };

        let now = now_iso();
        let source = Source {
            id,
            project_id: project.id,
            name,
            source_type,
            original_file_name: Some(original_file_name),
            original_path: Some(input_path),
            stored_path,
            extracted_path,
            byte_size: metadata.len(),
            extracted_char_count,
            enabled: true,
            parse_status,
            parse_message,
            created_at: now.clone(),
            updated_at: now,
        };
        if let Err(error) = self.insert_source(&source) {
            let _ = fs::remove_file(&source.stored_path);
            if let Some(path) = &source.extracted_path {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
        Ok(source)
    }

    pub fn create_pasted(&self, input: CreatePastedSourceInput) -> Result<Source, SourceError> {
        let project = self
            .project_storage
            .get_project(&input.project_id)
            .map_err(|error| SourceError::new(error.to_string()))?;
        let roots = ProjectSourceRoots::resolve(
            &self.project_storage.info().projects_root,
            &project.project_dir,
        )?;
        let text = normalize_text(input.text);
        if text.is_empty() {
            return Err(SourceError::new("粘贴文字不能为空"));
        }
        if text.len() > MAX_PASTED_BYTES {
            return Err(SourceError::new("粘贴文字超过 5 MB，MVP 暂不支持"));
        }
        let name = validate_source_name(input.name.as_deref().unwrap_or("粘贴文字"))?;
        let id = Uuid::new_v4().to_string();
        let stored_path = roots.sources.join(format!("{id}.txt"));
        let extracted_path = roots.extracted.join(format!("{id}.txt"));
        ensure_child_path(&stored_path, &roots.sources)?;
        ensure_child_path(&extracted_path, &roots.extracted)?;
        fs::write(&stored_path, text.as_bytes())?;
        if let Err(error) = fs::write(&extracted_path, text.as_bytes()) {
            let _ = fs::remove_file(&stored_path);
            return Err(SourceError::new(format!("保存提取文本失败：{error}")));
        }

        let now = now_iso();
        let source = Source {
            id,
            project_id: project.id,
            name,
            source_type: SourceType::PastedText,
            original_file_name: None,
            original_path: None,
            stored_path,
            extracted_path: Some(extracted_path),
            byte_size: text.len() as u64,
            extracted_char_count: text.chars().count() as u64,
            enabled: true,
            parse_status: SourceParseStatus::Ready,
            parse_message: None,
            created_at: now.clone(),
            updated_at: now,
        };
        if let Err(error) = self.insert_source(&source) {
            let _ = fs::remove_file(&source.stored_path);
            if let Some(path) = &source.extracted_path {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
        Ok(source)
    }

    pub fn read_text(&self, id: &str) -> Result<String, SourceError> {
        let source = self.get_source(id)?;
        let roots = self.roots_for_source(&source)?;
        let path = source.extracted_path.as_ref().ok_or_else(|| {
            SourceError::new(
                source
                    .parse_message
                    .clone()
                    .unwrap_or_else(|| "该资料没有可查看的提取文本".to_owned()),
            )
        })?;
        let canonical_path = fs::canonicalize(path)
            .map_err(|error| SourceError::new(format!("提取文本文件不可用：{error}")))?;
        let canonical_root = fs::canonicalize(&roots.extracted)
            .map_err(|error| SourceError::new(format!("提取文本目录不可用：{error}")))?;
        if canonical_path.parent() != Some(canonical_root.as_path()) || !canonical_path.is_file() {
            return Err(SourceError::new("提取文本路径越过了当前项目目录"));
        }
        let bytes = fs::read(canonical_path)?;
        String::from_utf8(bytes)
            .map_err(|error| SourceError::new(format!("提取文本不是有效的 UTF-8：{error}")))
    }

    pub fn set_enabled(&self, input: SetSourceEnabledInput) -> Result<Source, SourceError> {
        let source = self.get_source(&input.id)?;
        let now = now_iso();
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE sources SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
            params![input.id, input.enabled, now],
        )?;
        if changed != 1 {
            return Err(SourceError::new("资料不存在或已被删除"));
        }
        Ok(Source {
            enabled: input.enabled,
            updated_at: now,
            ..source
        })
    }

    pub fn delete(&self, id: &str) -> Result<(), SourceError> {
        let source = self.get_source(id)?;
        let roots = self.roots_for_source(&source)?;
        let stored_trash = stage_for_delete(&source.stored_path, &roots.sources, id)?;
        let extracted_trash = match &source.extracted_path {
            Some(path) => match stage_for_delete(path, &roots.extracted, id) {
                Ok(path) => path,
                Err(error) => {
                    restore_staged(&stored_trash, &source.stored_path);
                    return Err(error);
                }
            },
            None => None,
        };

        let delete_result = (|| -> Result<(), SourceError> {
            let connection = self.connection()?;
            let changed = connection.execute("DELETE FROM sources WHERE id = ?1", [id])?;
            if changed != 1 {
                return Err(SourceError::new("资料不存在或已被删除"));
            }
            Ok(())
        })();
        if let Err(error) = delete_result {
            restore_staged(&stored_trash, &source.stored_path);
            if let Some(path) = &source.extracted_path {
                restore_staged(&extracted_trash, path);
            }
            return Err(error);
        }
        remove_staged(stored_trash);
        remove_staged(extracted_trash);
        Ok(())
    }

    fn insert_source(&self, source: &Source) -> Result<(), SourceError> {
        let byte_size = i64::try_from(source.byte_size)
            .map_err(|_| SourceError::new("资料文件大小超出数据库范围"))?;
        let char_count = i64::try_from(source.extracted_char_count)
            .map_err(|_| SourceError::new("提取文本长度超出数据库范围"))?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO sources (
                id, project_id, name, source_type, original_file_name, original_path,
                stored_path, extracted_path, byte_size, extracted_char_count, enabled,
                parse_status, parse_message, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                source.id,
                source.project_id,
                source.name,
                source.source_type.as_str(),
                source.original_file_name,
                source.original_path.as_deref().map(path_to_string),
                path_to_string(&source.stored_path),
                source.extracted_path.as_deref().map(path_to_string),
                byte_size,
                char_count,
                source.enabled,
                source.parse_status.as_str(),
                source.parse_message,
                source.created_at,
                source.updated_at,
            ],
        )?;
        Ok(())
    }

    fn get_source(&self, id: &str) -> Result<Source, SourceError> {
        validate_uuid(id)?;
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT id, project_id, name, source_type, original_file_name, original_path,
                        stored_path, extracted_path, byte_size, extracted_char_count, enabled,
                        parse_status, parse_message, created_at, updated_at
                 FROM sources WHERE id = ?1",
                [id],
                source_from_row,
            )
            .optional()?
            .ok_or_else(|| SourceError::new("资料不存在或已被删除"))
    }

    fn roots_for_source(&self, source: &Source) -> Result<ProjectSourceRoots, SourceError> {
        let project = self
            .project_storage
            .get_project(&source.project_id)
            .map_err(|error| SourceError::new(error.to_string()))?;
        ProjectSourceRoots::resolve(
            &self.project_storage.info().projects_root,
            &project.project_dir,
        )
    }
}

struct ProjectSourceRoots {
    sources: PathBuf,
    extracted: PathBuf,
}

impl ProjectSourceRoots {
    fn resolve(projects_root: &Path, project_dir: &Path) -> Result<Self, SourceError> {
        let canonical_projects_root = fs::canonicalize(projects_root)
            .map_err(|error| SourceError::new(format!("项目根目录不可用：{error}")))?;
        let canonical_project_dir = fs::canonicalize(project_dir)
            .map_err(|error| SourceError::new(format!("项目目录不可用：{error}")))?;
        if canonical_project_dir == canonical_projects_root
            || !canonical_project_dir.starts_with(&canonical_projects_root)
        {
            return Err(SourceError::new("项目目录越过了知画项目根目录"));
        }
        let sources = project_dir.join("sources");
        let extracted = project_dir.join("extracted");
        fs::create_dir_all(&sources)?;
        fs::create_dir_all(&extracted)?;
        let canonical_sources = fs::canonicalize(&sources)?;
        let canonical_extracted = fs::canonicalize(&extracted)?;
        if !canonical_sources.starts_with(&canonical_project_dir)
            || !canonical_extracted.starts_with(&canonical_project_dir)
        {
            return Err(SourceError::new("资料目录越过了当前项目目录"));
        }
        Ok(Self { sources, extracted })
    }
}

enum ExtractedContent {
    Text(String),
    NoText(String),
}

fn extract_source(path: &Path, source_type: SourceType) -> Result<ExtractedContent, SourceError> {
    let text = match source_type {
        SourceType::Txt => decode_text_file(path)?,
        SourceType::Pdf => pdf_extract::extract_text(path)
            .map_err(|error| SourceError::new(format!("PDF 文本提取失败：{error}")))?,
        SourceType::Docx => extract_docx(path)?,
        SourceType::Pptx => extract_pptx(path)?,
        SourceType::Image => {
            return Ok(ExtractedContent::NoText(
                "图片已导入；当前 MVP 未启用 OCR，因此没有提取文本".to_owned(),
            ))
        }
        SourceType::PastedText => unreachable!("pasted text is stored directly"),
    };
    let text = normalize_text(text);
    if text.is_empty() {
        let message = if source_type == SourceType::Pdf {
            "PDF 中未提取到可用文本；扫描版 PDF 需要 OCR".to_owned()
        } else {
            "资料中未找到可提取的文本".to_owned()
        };
        return Ok(ExtractedContent::NoText(message));
    }
    if text.len() > MAX_EXTRACTED_BYTES {
        return Err(SourceError::new("提取文本超过 20 MB，已停止处理"));
    }
    Ok(ExtractedContent::Text(text))
}

fn decode_text_file(path: &Path) -> Result<String, SourceError> {
    let bytes = fs::read(path)?;
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(bytes[3..].to_vec())
            .map_err(|error| SourceError::new(format!("TXT 的 UTF-8 编码无效：{error}")));
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (text, had_errors) = UTF_16LE.decode_without_bom_handling(&bytes[2..]);
        if had_errors {
            return Err(SourceError::new("TXT 的 UTF-16LE 编码无效"));
        }
        return Ok(text.into_owned());
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let (text, had_errors) = UTF_16BE.decode_without_bom_handling(&bytes[2..]);
        if had_errors {
            return Err(SourceError::new("TXT 的 UTF-16BE 编码无效"));
        }
        return Ok(text.into_owned());
    }
    if bytes.iter().filter(|byte| **byte == 0).count() > bytes.len() / 20 {
        return Err(SourceError::new("TXT 看起来是二进制文件，无法提取文字"));
    }
    if let Ok(text) = std::str::from_utf8(&bytes) {
        return Ok(text.to_owned());
    }
    let (text, had_errors) = GB18030.decode_without_bom_handling(&bytes);
    if had_errors {
        return Err(SourceError::new(
            "TXT 编码无法识别；请保存为 UTF-8、UTF-16 或 GB18030 后重试",
        ));
    }
    Ok(text.into_owned())
}

fn extract_docx(path: &Path) -> Result<String, SourceError> {
    let bytes = fs::read(path)?;
    let mut archive = open_office_archive(&bytes, "DOCX")?;
    let xml = read_zip_entry(&mut archive, "word/document.xml", "DOCX 主文档")?;
    extract_xml_text(&xml, false)
}

fn extract_pptx(path: &Path) -> Result<String, SourceError> {
    let bytes = fs::read(path)?;
    let mut archive = open_office_archive(&bytes, "PPTX")?;
    let mut slides: Vec<(u32, String)> = archive
        .file_names()
        .filter_map(|name| slide_number(name).map(|number| (number, name.to_owned())))
        .collect();
    slides.sort_by_key(|(number, _)| *number);
    if slides.is_empty() {
        return Err(SourceError::new("PPTX 中没有找到幻灯片内容"));
    }
    let mut output = String::new();
    for (number, name) in slides {
        let xml = read_zip_entry(&mut archive, &name, "PPTX 幻灯片")?;
        let text = extract_xml_text(&xml, true)?;
        if !text.trim().is_empty() {
            if !output.is_empty() {
                output.push_str("\n\n");
            }
            output.push_str(&format!("[第 {number} 页]\n{text}"));
        }
        if output.len() > MAX_EXTRACTED_BYTES {
            return Err(SourceError::new("PPTX 提取文本超过 20 MB，已停止处理"));
        }
    }
    Ok(output)
}

fn open_office_archive<'a>(
    bytes: &'a [u8],
    label: &str,
) -> Result<ZipArchive<Cursor<&'a [u8]>>, SourceError> {
    let archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| SourceError::new(format!("{label} 文件结构无效：{error}")))?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(SourceError::new(format!(
            "{label} 包含过多文件，已停止处理"
        )));
    }
    Ok(archive)
}

fn read_zip_entry<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
    label: &str,
) -> Result<Vec<u8>, SourceError> {
    let file = archive
        .by_name(name)
        .map_err(|_| SourceError::new(format!("{label}缺少必要内容：{name}")))?;
    if file.size() > MAX_XML_BYTES {
        return Err(SourceError::new(format!(
            "{label}的 XML 超过 32 MB，已停止处理"
        )));
    }
    let mut xml = Vec::with_capacity(file.size() as usize);
    file.take(MAX_XML_BYTES + 1).read_to_end(&mut xml)?;
    if xml.len() as u64 > MAX_XML_BYTES {
        return Err(SourceError::new(format!(
            "{label}的 XML 超过 32 MB，已停止处理"
        )));
    }
    Ok(xml)
}

fn extract_xml_text(xml: &[u8], powerpoint: bool) -> Result<String, SourceError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut output = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) => {
                if element.name().local_name().as_ref() == b"t" {
                    in_text = true;
                }
            }
            Ok(Event::Empty(element)) => match element.name().local_name().as_ref() {
                b"tab" => output.push('\t'),
                b"br" => output.push('\n'),
                _ => {}
            },
            Ok(Event::Text(text)) if in_text => {
                let decoded = text.decode().map_err(|error| {
                    SourceError::new(format!("Office XML 文本编码无效：{error}"))
                })?;
                let unescaped = quick_xml::escape::unescape(&decoded).map_err(|error| {
                    SourceError::new(format!("Office XML 转义文本无效：{error}"))
                })?;
                output.push_str(&unescaped);
            }
            Ok(Event::GeneralRef(reference)) if in_text => {
                let reference = reference.decode().map_err(|error| {
                    SourceError::new(format!("Office XML 实体编码无效：{error}"))
                })?;
                let escaped = format!("&{reference};");
                let unescaped = quick_xml::escape::unescape(&escaped)
                    .map_err(|error| SourceError::new(format!("Office XML 实体无效：{error}")))?;
                output.push_str(&unescaped);
            }
            Ok(Event::End(element)) => match element.name().local_name().as_ref() {
                b"t" => in_text = false,
                b"p" => {
                    if !output.ends_with('\n') {
                        output.push('\n');
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(SourceError::new(format!(
                    "Office XML 解析失败（位置 {}）：{error}",
                    reader.error_position()
                )))
            }
            _ => {}
        }
        if output.len() > MAX_EXTRACTED_BYTES {
            return Err(SourceError::new("Office 提取文本超过 20 MB，已停止处理"));
        }
        buffer.clear();
    }
    if powerpoint && output.ends_with('\n') {
        output.pop();
    }
    Ok(output)
}

fn source_type_for_extension(extension: &str) -> Result<SourceType, SourceError> {
    match extension {
        "txt" => Ok(SourceType::Txt),
        "pdf" => Ok(SourceType::Pdf),
        "docx" => Ok(SourceType::Docx),
        "pptx" => Ok(SourceType::Pptx),
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" => Ok(SourceType::Image),
        _ => Err(SourceError::new(
            "不支持该文件类型；请选择 TXT、PDF、DOCX、PPTX、PNG、JPG、WEBP、BMP 或 GIF",
        )),
    }
}

fn validate_file_signature(
    path: &Path,
    source_type: SourceType,
    extension: &str,
) -> Result<(), SourceError> {
    let mut file = fs::File::open(path)?;
    let mut header = [0_u8; 16];
    let count = file.read(&mut header)?;
    let header = &header[..count];
    let valid = match source_type {
        SourceType::Txt => true,
        SourceType::Pdf => header.starts_with(b"%PDF-"),
        SourceType::Docx | SourceType::Pptx => {
            header.starts_with(b"PK\x03\x04")
                || header.starts_with(b"PK\x05\x06")
                || header.starts_with(b"PK\x07\x08")
        }
        SourceType::Image => match extension {
            "png" => header.starts_with(b"\x89PNG\r\n\x1a\n"),
            "jpg" | "jpeg" => header.starts_with(b"\xFF\xD8\xFF"),
            "webp" => header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP",
            "bmp" => header.starts_with(b"BM"),
            "gif" => header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a"),
            _ => false,
        },
        SourceType::PastedText => true,
    };
    if valid {
        Ok(())
    } else {
        Err(SourceError::new(format!(
            "文件内容与 .{extension} 扩展名不匹配"
        )))
    }
}

fn slide_number(name: &str) -> Option<u32> {
    name.strip_prefix("ppt/slides/slide")?
        .strip_suffix(".xml")?
        .parse()
        .ok()
}

fn source_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Source> {
    let source_type_text: String = row.get(3)?;
    let parse_status_text: String = row.get(11)?;
    let source_type = SourceType::from_str(&source_type_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let parse_status = SourceParseStatus::from_str(&parse_status_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(11, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let byte_size: i64 = row.get(8)?;
    let char_count: i64 = row.get(9)?;
    Ok(Source {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        source_type,
        original_file_name: row.get(4)?,
        original_path: row.get::<_, Option<String>>(5)?.map(PathBuf::from),
        stored_path: PathBuf::from(row.get::<_, String>(6)?),
        extracted_path: row.get::<_, Option<String>>(7)?.map(PathBuf::from),
        byte_size: u64::try_from(byte_size).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
        extracted_char_count: u64::try_from(char_count).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
        enabled: row.get(10)?,
        parse_status,
        parse_message: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

fn validate_source_name(name: &str) -> Result<String, SourceError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(SourceError::new("资料名称不能为空"));
    }
    if name.chars().count() > 200 {
        return Err(SourceError::new("资料名称不能超过 200 个字符"));
    }
    if name.chars().any(char::is_control) {
        return Err(SourceError::new("资料名称不能包含控制字符"));
    }
    Ok(name.to_owned())
}

fn validate_uuid(id: &str) -> Result<(), SourceError> {
    Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| SourceError::new("资料 ID 格式无效"))
}

fn ensure_child_path(path: &Path, root: &Path) -> Result<(), SourceError> {
    if path.parent() == Some(root) {
        Ok(())
    } else {
        Err(SourceError::new("资料文件路径越过了当前项目目录"))
    }
}

fn stage_for_delete(path: &Path, root: &Path, id: &str) -> Result<Option<PathBuf>, SourceError> {
    if !path.exists() {
        return Ok(None);
    }
    let canonical = fs::canonicalize(path)?;
    let canonical_root = fs::canonicalize(root)?;
    if !canonical.is_file() || canonical.parent() != Some(canonical_root.as_path()) {
        return Err(SourceError::new("拒绝删除项目目录之外的资料文件"));
    }
    let suffix = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("tmp");
    let trash = root.join(format!(".deleting-{id}.{suffix}"));
    ensure_child_path(&trash, root)?;
    if trash.exists() {
        return Err(SourceError::new("资料删除暂存文件已存在，请重试"));
    }
    fs::rename(&canonical, &trash)?;
    Ok(Some(trash))
}

fn restore_staged(staged: &Option<PathBuf>, original: &Path) {
    if let Some(staged) = staged {
        let _ = fs::rename(staged, original);
    }
}

fn remove_staged(staged: Option<PathBuf>) {
    if let Some(staged) = staged {
        let _ = fs::remove_file(staged);
    }
}

fn normalize_text(text: String) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_owned()
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::CreateProjectInput;
    use std::io::Write;
    use zip::{write::SimpleFileOptions, ZipWriter};

    fn storage() -> (tempfile::TempDir, SourceStorage, String, PathBuf) {
        let directory = tempfile::tempdir().expect("temporary directory");
        let project_storage = ProjectStorage::initialize(
            directory.path().join("zhihua.sqlite3"),
            directory.path().join("projects"),
        )
        .expect("initialize project storage");
        let project = project_storage
            .create_project(CreateProjectInput {
                title: "资料测试".to_owned(),
                audience: None,
                target_duration_sec: None,
            })
            .expect("create project");
        let source_storage =
            SourceStorage::initialize(project_storage).expect("initialize source storage");
        (directory, source_storage, project.id, project.project_dir)
    }

    fn make_office_file(path: &Path, entries: &[(&str, &str)]) {
        let file = fs::File::create(path).expect("create zip");
        let mut writer = ZipWriter::new(file);
        for (name, contents) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .expect("start zip entry");
            writer
                .write_all(contents.as_bytes())
                .expect("write zip entry");
        }
        writer.finish().expect("finish zip");
    }

    #[test]
    fn imports_utf8_and_gb18030_txt_and_reads_real_extracted_text() {
        let (directory, storage, project_id, project_dir) = storage();
        let utf8_path = directory.path().join("雷电.txt");
        fs::write(&utf8_path, "电荷分离\r\n形成电场。\n").expect("write txt");
        let source = storage
            .import_file(ImportSourceFileInput {
                project_id: project_id.clone(),
                file_path: utf8_path,
                name: None,
            })
            .expect("import utf8 txt");
        assert_eq!(source.parse_status, SourceParseStatus::Ready);
        assert_eq!(
            storage.read_text(&source.id).expect("read text"),
            "电荷分离\n形成电场。"
        );
        assert!(source.stored_path.starts_with(project_dir.join("sources")));
        assert!(source
            .extracted_path
            .as_ref()
            .expect("extracted path")
            .starts_with(project_dir.join("extracted")));

        let gb_path = directory.path().join("gb.txt");
        let (encoded, _, _) = GB18030.encode("雷声来自空气快速膨胀");
        fs::write(&gb_path, encoded).expect("write gb18030");
        let gb = storage
            .import_file(ImportSourceFileInput {
                project_id,
                file_path: gb_path,
                name: Some("GB 文本".to_owned()),
            })
            .expect("import gb18030");
        assert_eq!(
            storage.read_text(&gb.id).expect("read gb text"),
            "雷声来自空气快速膨胀"
        );
    }

    #[test]
    fn extracts_docx_and_pptx_xml_in_document_order() {
        let (directory, storage, project_id, _project_dir) = storage();
        let docx_path = directory.path().join("lesson.docx");
        make_office_file(
            &docx_path,
            &[(
                "word/document.xml",
                r#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body><w:p><w:r><w:t>云层中的电荷</w:t></w:r></w:p><w:p><w:r><w:t>形成电场 &amp; 闪电</w:t></w:r></w:p></w:body></w:document>"#,
            )],
        );
        let docx = storage
            .import_file(ImportSourceFileInput {
                project_id: project_id.clone(),
                file_path: docx_path,
                name: None,
            })
            .expect("import docx");
        assert_eq!(
            storage.read_text(&docx.id).expect("read docx"),
            "云层中的电荷\n形成电场 & 闪电"
        );

        let pptx_path = directory.path().join("lesson.pptx");
        make_office_file(
            &pptx_path,
            &[
                (
                    "ppt/slides/slide2.xml",
                    r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>第二页</a:t></a:r></a:p></p:sld>"#,
                ),
                (
                    "ppt/slides/slide1.xml",
                    r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>第一页</a:t></a:r></a:p></p:sld>"#,
                ),
            ],
        );
        let pptx = storage
            .import_file(ImportSourceFileInput {
                project_id,
                file_path: pptx_path,
                name: None,
            })
            .expect("import pptx");
        assert_eq!(
            storage.read_text(&pptx.id).expect("read pptx"),
            "[第 1 页]\n第一页\n\n[第 2 页]\n第二页"
        );
    }

    #[test]
    fn records_honest_failure_for_invalid_office_content() {
        let (directory, storage, project_id, _project_dir) = storage();
        let path = directory.path().join("broken.docx");
        make_office_file(&path, &[("other.xml", "<root/>")]);
        let source = storage
            .import_file(ImportSourceFileInput {
                project_id,
                file_path: path,
                name: None,
            })
            .expect("store source with failed parse state");
        assert_eq!(source.parse_status, SourceParseStatus::Failed);
        assert!(source
            .parse_message
            .as_deref()
            .expect("parse message")
            .contains("word/document.xml"));
        assert!(storage.read_text(&source.id).is_err());
    }

    #[test]
    fn creates_toggles_lists_and_deletes_pasted_text() {
        let (_directory, storage, project_id, project_dir) = storage();
        let source = storage
            .create_pasted(CreatePastedSourceInput {
                project_id: project_id.clone(),
                name: Some("补充说明".to_owned()),
                text: "  先看见闪电，后听见雷声。  ".to_owned(),
            })
            .expect("create pasted source");
        assert_eq!(source.source_type, SourceType::PastedText);
        assert_eq!(
            storage.list_sources(&project_id).expect("list"),
            vec![source.clone()]
        );
        let disabled = storage
            .set_enabled(SetSourceEnabledInput {
                id: source.id.clone(),
                enabled: false,
            })
            .expect("disable");
        assert!(!disabled.enabled);
        storage.delete(&source.id).expect("delete source");
        assert!(storage
            .list_sources(&project_id)
            .expect("list empty")
            .is_empty());
        assert!(project_dir
            .join("sources")
            .read_dir()
            .expect("sources dir")
            .next()
            .is_none());
        assert!(project_dir
            .join("extracted")
            .read_dir()
            .expect("extracted dir")
            .next()
            .is_none());
    }

    #[test]
    fn rejects_unsupported_or_spoofed_files_before_copying() {
        let (directory, storage, project_id, project_dir) = storage();
        let fake_pdf = directory.path().join("fake.pdf");
        fs::write(&fake_pdf, "not a pdf").expect("write fake pdf");
        let error = storage
            .import_file(ImportSourceFileInput {
                project_id: project_id.clone(),
                file_path: fake_pdf,
                name: None,
            })
            .expect_err("reject spoofed pdf");
        assert!(error.to_string().contains("不匹配"));

        let executable = directory.path().join("unsafe.exe");
        fs::write(&executable, "MZ").expect("write exe");
        let error = storage
            .import_file(ImportSourceFileInput {
                project_id,
                file_path: executable,
                name: None,
            })
            .expect_err("reject unsupported extension");
        assert!(error.to_string().contains("不支持"));
        assert!(project_dir
            .join("sources")
            .read_dir()
            .expect("sources dir")
            .next()
            .is_none());
    }
}
