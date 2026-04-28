use clap::{Parser, Subcommand};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;

#[derive(Parser)]
#[command(name = "filesindex")]
#[command(about = "CLI utility for indexing and searching files by tags")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Add {
        #[arg(long)]
        path: String,

        #[arg(long)]
        tags: String,
    },
    Get {
        #[arg(long)]
        tags: String,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct FileRecord {
    path: String,
    tags: Vec<String>,
}

trait IndexStorage {
    fn add_file(&mut self, record: FileRecord) -> Result<(), String>;
    fn get_files_by_tags(&self, tags: Vec<String>) -> Result<Vec<FileRecord>, String>;
}

struct JsonStorage {
    file_path: String,
}

impl JsonStorage {
    fn new(file_path: String) -> Self {
        JsonStorage { file_path }
    }

    fn load_records(&self) -> Result<Vec<FileRecord>, String> {
        if !Path::new(&self.file_path).exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.file_path)
            .map_err(|error| format!("Помилка читання JSON файлу: {}", error))?;

        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        serde_json::from_str(&content)
            .map_err(|error| format!("Помилка парсингу JSON: {}", error))
    }

    fn save_records(&self, records: &[FileRecord]) -> Result<(), String> {
        let content = serde_json::to_string_pretty(records)
            .map_err(|error| format!("Помилка серіалізації JSON: {}", error))?;

        fs::write(&self.file_path, content)
            .map_err(|error| format!("Помилка запису JSON файлу: {}", error))
    }
}

impl IndexStorage for JsonStorage {
    fn add_file(&mut self, record: FileRecord) -> Result<(), String> {
        let mut records = self.load_records()?;

        records.retain(|item| item.path != record.path);
        records.push(record);

        self.save_records(&records)
    }

    fn get_files_by_tags(&self, tags: Vec<String>) -> Result<Vec<FileRecord>, String> {
        let records = self.load_records()?;
        Ok(filter_records_by_tags(records, tags))
    }
}

struct SqliteStorage {
    db_path: String,
}

impl SqliteStorage {
    fn new(db_path: String) -> Self {
        SqliteStorage { db_path }
    }

    fn connect(&self) -> Result<Connection, String> {
        let conn = Connection::open(&self.db_path)
            .map_err(|error| format!("Помилка відкриття SQLite бази: {}", error))?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                tags TEXT NOT NULL
            )",
            [],
        )
        .map_err(|error| format!("Помилка створення таблиці: {}", error))?;

        Ok(conn)
    }
}

impl IndexStorage for SqliteStorage {
    fn add_file(&mut self, record: FileRecord) -> Result<(), String> {
        let conn = self.connect()?;
        let tags_json = serde_json::to_string(&record.tags)
            .map_err(|error| format!("Помилка серіалізації тегів: {}", error))?;

        conn.execute(
            "INSERT INTO files (path, tags)
             VALUES (?1, ?2)
             ON CONFLICT(path) DO UPDATE SET tags = excluded.tags",
            params![record.path, tags_json],
        )
        .map_err(|error| format!("Помилка запису в SQLite: {}", error))?;

        Ok(())
    }

    fn get_files_by_tags(&self, tags: Vec<String>) -> Result<Vec<FileRecord>, String> {
        let conn = self.connect()?;

        let mut stmt = conn
            .prepare("SELECT path, tags FROM files")
            .map_err(|error| format!("Помилка SQL-запиту: {}", error))?;

        let rows = stmt
            .query_map([], |row| {
                let path: String = row.get(0)?;
                let tags_json: String = row.get(1)?;

                let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();

                Ok(FileRecord { path, tags })
            })
            .map_err(|error| format!("Помилка читання з SQLite: {}", error))?;

        let mut records = Vec::new();

        for row in rows {
            records.push(row.map_err(|error| format!("Помилка обробки рядка: {}", error))?);
        }

        Ok(filter_records_by_tags(records, tags))
    }
}

fn filter_records_by_tags(records: Vec<FileRecord>, search_tags: Vec<String>) -> Vec<FileRecord> {
    let search_set: HashSet<String> = search_tags.into_iter().collect();

    records
        .into_iter()
        .filter(|record| {
            let record_tags: HashSet<String> = record.tags.iter().cloned().collect();
            search_set.iter().all(|tag| record_tags.contains(tag))
        })
        .collect()
}

fn parse_tags(tags: &str) -> Vec<String> {
    tags.split(',')
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect()
}

fn create_storage() -> Result<Box<dyn IndexStorage>, String> {
    let config = env::var("FILES_INDEX_PATH")
        .map_err(|_| "Не задано змінну середовища FILES_INDEX_PATH".to_string())?;

    let parts: Vec<&str> = config.splitn(2, ':').collect();

    if parts.len() != 2 {
        return Err("Неправильний формат FILES_INDEX_PATH. Приклад: json:index.json".to_string());
    }

    let storage_type = parts[0];
    let path = parts[1].to_string();

    match storage_type {
        "json" => Ok(Box::new(JsonStorage::new(path))),
        "sqlite" => Ok(Box::new(SqliteStorage::new(path))),
        _ => Err("Невідомий тип сховища. Використайте json або sqlite".to_string()),
    }
}

fn add_file_static<S: IndexStorage>(
    storage: &mut S,
    path: String,
    tags: Vec<String>,
) -> Result<(), String> {
    let record = FileRecord { path, tags };
    storage.add_file(record)
}

fn print_records(records: Vec<FileRecord>) {
    if records.is_empty() {
        println!("Файли з такими тегами не знайдено.");
        return;
    }

    println!("Знайдені файли:");

    for record in records {
        println!("Файл: {}", record.path);
        println!("Теги: {}", record.tags.join(", "));
        println!("------------------------");
    }
}

fn main() {
    let cli = Cli::parse();

    let mut storage = match create_storage() {
        Ok(storage) => storage,
        Err(error) => {
            eprintln!("{}", error);
            return;
        }
    };

    match cli.command {
        Commands::Add { path, tags } => {
            let parsed_tags = parse_tags(&tags);
            let record = FileRecord {
                path,
                tags: parsed_tags,
            };

            match storage.add_file(record) {
                Ok(_) => println!("Файл успішно додано до індексу."),
                Err(error) => eprintln!("{}", error),
            }
        }

        Commands::Get { tags } => {
            let parsed_tags = parse_tags(&tags);

            match storage.get_files_by_tags(parsed_tags) {
                Ok(records) => print_records(records),
                Err(error) => eprintln!("{}", error),
            }
        }
    }
}