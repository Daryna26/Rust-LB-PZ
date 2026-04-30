#![warn(missing_docs)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::missing_errors_doc)]
#![warn(clippy::result_large_err)]

//! CLI-застосунок для зміни розміру зображень.
//!
//! Програма читає список шляхів або URL із файлу,
//! змінює розмір зображень і зберігає результат
//! у файлову систему або у S3-сховище.

use clap::Parser;
use image::imageops::FilterType;
use image::ImageFormat;
use reqwest::blocking::Client;
use std::env;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use thiserror::Error;

/// Основний тип помилок застосунку.
#[derive(Debug, Error)]
pub enum AppError {
    /// Помилка вводу/виводу.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Помилка HTTP-запиту.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// Помилка обробки зображення.
    #[error("Image error: {0}")]
    Image(#[from] image::ImageError),

    /// Помилка змінної середовища.
    #[error("Environment error: {0}")]
    Env(#[from] std::env::VarError),

    /// Неправильний формат розміру.
    #[error("Invalid resize format. Use widthxheight")]
    InvalidResizeFormat,

    /// Помилка перетворення числа.
    #[error("Parse error: {0}")]
    Parse(#[from] std::num::ParseIntError),

    /// Невідомий тип uploader.
    #[error("Unknown uploader")]
    UnknownUploader,

    /// Помилка завантаження у S3.
    #[error("S3 upload failed: {0}")]
    S3Upload(String),
}

/// Тип результату застосунку.
type AppResult<T> = Result<T, AppError>;

/// CLI аргументи.
#[derive(Parser)]
#[command(name = "image_editor")]
#[command(about = "Resize images and upload them to FS or S3")]
struct Cli {
    /// Шлях до файлу зі списком зображень
    #[arg(long)]
    files: String,

    /// Новий розмір (наприклад 300x300)
    #[arg(long)]
    resize: String,
}

/// Трейт для збереження файлів.
trait Uploader {
    /// Завантажує файл.
    ///
    /// # Errors
    /// Повертає помилку якщо не вдалося зберегти файл.
    fn upload(&self, file_name: &str, data: &[u8]) -> AppResult<()>;
}

/// Збереження у файлову систему.
struct FsUploader {
    dir: PathBuf,
}

impl FsUploader {
    /// Створює uploader для файлової системи.
    ///
    /// # Errors
    /// Якщо змінна середовища відсутня або директорію не створено.
    fn new() -> AppResult<Self> {
        let dir = PathBuf::from(env::var("MYME_FILES_PATH")?);
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }
}

impl Uploader for FsUploader {
    fn upload(&self, file_name: &str, data: &[u8]) -> AppResult<()> {
        let path = self.dir.join(file_name);
        fs::write(&path, data)?;
        println!("Saved: {}", path.display());
        Ok(())
    }
}

/// Збереження у S3.
struct S3Uploader {
    endpoint: String,
    bucket: String,
    access_key: String,
    secret_key: String,
    client: Client,
}

impl S3Uploader {
    /// Створює S3 uploader.
    ///
    /// # Errors
    /// Якщо відсутні змінні середовища.
    fn new() -> AppResult<Self> {
        Ok(Self {
            endpoint: env::var("S3_ENDPOINT")?,
            bucket: env::var("S3_BUCKET")?,
            access_key: env::var("S3_ACCESS_KEY_ID")?,
            secret_key: env::var("S3_SECRET_ACCESS_KEY")?,
            client: Client::new(),
        })
    }
}

impl Uploader for S3Uploader {
    fn upload(&self, file_name: &str, data: &[u8]) -> AppResult<()> {
        let url = format!("{}/{}/{}", self.endpoint, self.bucket, file_name);

        let res = self
            .client
            .put(&url)
            .body(data.to_vec())
            .send()?;

        if !res.status().is_success() {
            return Err(AppError::S3Upload(res.status().to_string()));
        }

        println!("Uploaded: {}", file_name);
        Ok(())
    }
}

/// Створює uploader залежно від змінної середовища.
///
/// # Errors
/// Якщо тип uploader невідомий.
fn create_uploader() -> AppResult<Box<dyn Uploader>> {
    match env::var("MYME_UPLOADER").unwrap_or("fs".into()).as_str() {
        "fs" => Ok(Box::new(FsUploader::new()?)),
        "s3" => Ok(Box::new(S3Uploader::new()?)),
        _ => Err(AppError::UnknownUploader),
    }
}

/// Парсить розмір.
///
/// # Errors
/// Якщо формат неправильний.
fn parse_resize(s: &str) -> AppResult<(u32, u32)> {
    let parts: Vec<&str> = s.split('x').collect();

    if parts.len() != 2 {
        return Err(AppError::InvalidResizeFormat);
    }

    Ok((parts[0].parse()?, parts[1].parse()?))
}

/// Перевіряє чи це URL.
fn is_url(s: &str) -> bool {
    s.starts_with("http")
}

/// Завантажує зображення.
///
/// # Errors
/// Якщо не вдалося прочитати файл або URL.
fn load_image(source: &str) -> AppResult<Vec<u8>> {
    if is_url(source) {
        let bytes = reqwest::blocking::get(source)?.bytes()?;
        Ok(bytes.to_vec())
    } else {
        Ok(fs::read(source)?)
    }
}

/// Обробляє одне зображення.
///
/// # Errors
/// Якщо обробка або збереження не вдалося.
fn process_image(
    src: &str,
    w: u32,
    h: u32,
    i: usize,
    uploader: &dyn Uploader,
) -> AppResult<()> {
    let img = image::load_from_memory(&load_image(src)?)?;
    let resized = img.resize_exact(w, h, FilterType::Lanczos3);

    let mut buf = Cursor::new(Vec::new());
    resized.write_to(&mut buf, ImageFormat::Png)?;

    uploader.upload(&format!("{}_out.png", i), &buf.into_inner())?;
    Ok(())
}

/// Основна логіка програми.
///
/// # Errors
/// Якщо не вдалося прочитати файл або обробити зображення.
fn run() -> AppResult<()> {
    let cli = Cli::parse();
    let (w, h) = parse_resize(&cli.resize)?;
    let uploader = create_uploader()?;

    let content = fs::read_to_string(&cli.files)?;

    for (i, line) in content.lines().enumerate() {
        let line = line.trim();
        if !line.is_empty() {
            process_image(line, w, h, i, uploader.as_ref())?;
        }
    }

    Ok(())
}

/// Точка входу.
fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e}");
    }
}