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
use rayon::prelude::*;
use reqwest::blocking::Client;
use std::env;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::Instant;
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
    /// Шлях до файлу зі списком зображень.
    #[arg(long)]
    files: String,

    /// Новий розмір, наприклад 300x300.
    #[arg(long)]
    resize: String,
}

/// Трейт для збереження файлів.
trait Uploader {
    /// Завантажує або зберігає файл.
    ///
    /// # Errors
    ///
    /// Повертає помилку, якщо не вдалося зберегти файл.
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
    ///
    /// Повертає помилку, якщо відсутня змінна середовища
    /// або директорію неможливо створити.
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

/// Збереження у S3-compatible сховище.
struct S3Uploader {
    endpoint: String,
    bucket: String,
    client: Client,
}

impl S3Uploader {
    /// Створює S3 uploader.
    ///
    /// # Errors
    ///
    /// Повертає помилку, якщо відсутні потрібні змінні середовища.
    fn new() -> AppResult<Self> {
        Ok(Self {
            endpoint: env::var("S3_ENDPOINT")?,
            bucket: env::var("S3_BUCKET")?,
            client: Client::new(),
        })
    }
}

impl Uploader for S3Uploader {
    fn upload(&self, file_name: &str, data: &[u8]) -> AppResult<()> {
        let url = format!(
            "{}/{}/{}",
            self.endpoint.trim_end_matches('/'),
            self.bucket,
            file_name
        );

        let response = self.client.put(&url).body(data.to_vec()).send()?;

        if !response.status().is_success() {
            return Err(AppError::S3Upload(response.status().to_string()));
        }

        println!("Uploaded: {file_name}");
        Ok(())
    }
}

/// Створює uploader залежно від змінної середовища.
///
/// # Errors
///
/// Повертає помилку, якщо тип uploader невідомий.
fn create_uploader() -> AppResult<Box<dyn Uploader>> {
    match env::var("MYME_UPLOADER")
        .unwrap_or_else(|_| "fs".to_string())
        .as_str()
    {
        "fs" => Ok(Box::new(FsUploader::new()?)),
        "s3" => Ok(Box::new(S3Uploader::new()?)),
        _ => Err(AppError::UnknownUploader),
    }
}

/// Парсить розмір.
///
/// # Errors
///
/// Повертає помилку, якщо формат неправильний.
fn parse_resize(value: &str) -> AppResult<(u32, u32)> {
    let parts: Vec<&str> = value.split('x').collect();

    if parts.len() != 2 {
        return Err(AppError::InvalidResizeFormat);
    }

    let width = parts[0].parse::<u32>()?;
    let height = parts[1].parse::<u32>()?;

    if width == 0 || height == 0 {
        return Err(AppError::InvalidResizeFormat);
    }

    Ok((width, height))
}

/// Перевіряє, чи рядок є URL.
fn is_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

/// Завантажує зображення з URL або читає локальний файл.
///
/// # Errors
///
/// Повертає помилку, якщо не вдалося прочитати або завантажити зображення.
fn load_image(source: &str) -> AppResult<Vec<u8>> {
    if is_url(source) {
        let bytes = reqwest::blocking::get(source)?.bytes()?;
        Ok(bytes.to_vec())
    } else {
        Ok(fs::read(source)?)
    }
}

/// Створює назву вихідного файлу.
fn create_output_name(index: usize) -> String {
    format!("resized_image_{index}.png")
}

/// Виконує CPU-bound обробку зображення.
///
/// # Errors
///
/// Повертає помилку, якщо зображення неможливо прочитати, декодувати,
/// змінити або закодувати.
fn process_image_cpu(source: &str, width: u32, height: u32, index: usize) -> AppResult<(String, Vec<u8>)> {
    let image_bytes = load_image(source)?;
    let image = image::load_from_memory(&image_bytes)?;
    let resized = image.resize_exact(width, height, FilterType::Lanczos3);

    let mut buffer = Cursor::new(Vec::new());
    resized.write_to(&mut buffer, ImageFormat::Png)?;

    let output_name = create_output_name(index);

    Ok((output_name, buffer.into_inner()))
}

fn run() -> AppResult<()> {
    let cli = Cli::parse();

    let (width, height) = parse_resize(&cli.resize)?;
    let uploader = create_uploader()?;

    let content = fs::read_to_string(&cli.files)?;

    let sources: Vec<String> = content
        .lines()
        .map(|line| line.trim().trim_matches('"').to_string())
        .filter(|line| !line.is_empty())
        .collect();

    if sources.is_empty() {
        println!("Image list is empty. Nothing to process.");
        return Ok(());
    }

    let start = Instant::now();

    let results: Vec<AppResult<(String, Vec<u8>)>> = sources
        .par_iter()
        .enumerate()
        .map(|(index, source)| process_image_cpu(source, width, height, index + 1))
        .collect();

    for result in results {
        match result {
            Ok((file_name, data)) => {
                uploader.upload(&file_name, &data)?;
            }
            Err(error) => {
                eprintln!("Image processing error: {error}");
            }
        }
    }

    let elapsed = start.elapsed();
    println!("Processing finished in: {:?}", elapsed);

    Ok(())
}


fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
    }
}