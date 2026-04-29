#![warn(clippy::missing_errors_doc, clippy::result_large_err)]

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileRecord {
    pub path: String,
    pub tags: Vec<String>,
}

pub trait IndexStorage {

    fn add_file(&mut self, record: FileRecord) -> Result<(), IndexError>;


    fn get_files_by_tags(&self, tags: Vec<String>) -> Result<Vec<FileRecord>, IndexError>;
}

pub struct JsonStorage {
    file_path: String,
}

impl JsonStorage {
    #[must_use]
    pub fn new(file_path: String) -> Self {
        Self { file_path }
    }

    pub fn load_records(&self) -> Result<Vec<FileRecord>, IndexError> {
        if !Path::new(&self.file_path).exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.file_path)?;

        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        let records = serde_json::from_str(&content)?;

        Ok(records)
    }


    pub fn save_records(&self, records: &[FileRecord]) -> Result<(), IndexError> {
        let content = serde_json::to_string_pretty(records)?;
        fs::write(&self.file_path, content)?;

        Ok(())
    }
}

impl IndexStorage for JsonStorage {
    fn add_file(&mut self, record: FileRecord) -> Result<(), IndexError> {
        let mut records = self.load_records()?;

        records.retain(|item| item.path != record.path);
        records.push(record);

        self.save_records(&records)
    }

    fn get_files_by_tags(&self, tags: Vec<String>) -> Result<Vec<FileRecord>, IndexError> {
        let records = self.load_records()?;

        Ok(filter_records_by_tags(records, tags))
    }
}

pub struct SqliteStorage {
    db_path: String,
}

impl SqliteStorage {
    #[must_use]
    pub fn new(db_path: String) -> Self {
        Self { db_path }
    }


    pub fn connect(&self) -> Result<Connection, IndexError> {
        let conn = Connection::open(&self.db_path)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                tags TEXT NOT NULL
            )",
            [],
        )?;

        Ok(conn)
    }
}

impl IndexStorage for SqliteStorage {
    fn add_file(&mut self, record: FileRecord) -> Result<(), IndexError> {
        let conn = self.connect()?;
        let tags_json = serde_json::to_string(&record.tags)?;

        conn.execute(
            "INSERT INTO files (path, tags)
             VALUES (?1, ?2)
             ON CONFLICT(path) DO UPDATE SET tags = excluded.tags",
            params![record.path, tags_json],
        )?;

        Ok(())
    }

    fn get_files_by_tags(&self, tags: Vec<String>) -> Result<Vec<FileRecord>, IndexError> {
        let conn = self.connect()?;

        let mut stmt = conn.prepare("SELECT path, tags FROM files")?;

        let rows = stmt.query_map([], |row| {
            let path: String = row.get(0)?;
            let tags_json: String = row.get(1)?;

            Ok((path, tags_json))
        })?;

        let mut records = Vec::new();

        for row in rows {
            let (path, tags_json) = row?;
            let tags: Vec<String> = serde_json::from_str(&tags_json)?;

            records.push(FileRecord { path, tags });
        }

        Ok(filter_records_by_tags(records, tags))
    }
}

#[must_use]
pub fn filter_records_by_tags(
    records: Vec<FileRecord>,
    search_tags: Vec<String>,
) -> Vec<FileRecord> {
    let search_set: HashSet<String> = search_tags.into_iter().collect();

    records
        .into_iter()
        .filter(|record| {
            let record_tags: HashSet<String> = record.tags.iter().cloned().collect();

            search_set.iter().all(|tag| record_tags.contains(tag))
        })
        .collect()
}

#[must_use]
pub fn parse_tags(tags: &str) -> Vec<String> {
    tags.split(',')
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect()
}