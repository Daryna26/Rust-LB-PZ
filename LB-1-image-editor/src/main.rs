use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use hmac::{Hmac, Mac};
use image::imageops::FilterType;
use image::ImageFormat;
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

type HmacSha256 = Hmac<Sha256>;

#[derive(Parser)]
#[command(name = "image_editor")]
#[command(about = "CLI program for resizing images and uploading them to FS or S3")]
struct Cli {
    #[arg(long)]
    files: String,

    #[arg(long)]
    resize: String,
}

trait Uploader {
    fn upload(&self, file_name: &str, data: &[u8]) -> Result<()>;
}

struct FsUploader {
    output_dir: PathBuf,
}

impl FsUploader {
    fn new() -> Result<Self> {
        let output_dir = env::var("MYME_FILES_PATH")
            .context("Environment variable MYME_FILES_PATH is not set")?;

        let output_dir = PathBuf::from(output_dir);

        if !output_dir.exists() {
            fs::create_dir_all(&output_dir)
                .context("Cannot create output directory")?;
        }

        Ok(Self { output_dir })
    }
}

impl Uploader for FsUploader {
    fn upload(&self, file_name: &str, data: &[u8]) -> Result<()> {
        let output_path = self.output_dir.join(file_name);

        fs::write(&output_path, data)
            .with_context(|| format!("Cannot save file to {}", output_path.display()))?;

        println!("Saved to filesystem: {}", output_path.display());

        Ok(())
    }
}

struct S3Uploader {
    endpoint: String,
    bucket: String,
    region: String,
    access_key: String,
    secret_key: String,
    client: Client,
}

impl S3Uploader {
    fn new() -> Result<Self> {
        Ok(Self {
            endpoint: env::var("S3_ENDPOINT")
                .context("Environment variable S3_ENDPOINT is not set")?,
            bucket: env::var("S3_BUCKET")
                .context("Environment variable S3_BUCKET is not set")?,
            region: env::var("S3_REGION")
                .unwrap_or_else(|_| "auto".to_string()),
            access_key: env::var("S3_ACCESS_KEY_ID")
                .context("Environment variable S3_ACCESS_KEY_ID is not set")?,
            secret_key: env::var("S3_SECRET_ACCESS_KEY")
                .context("Environment variable S3_SECRET_ACCESS_KEY is not set")?,
            client: Client::new(),
        })
    }

    fn sign_key(&self, date: &str) -> Result<Vec<u8>> {
        let k_date = hmac_sha256(format!("AWS4{}", self.secret_key).as_bytes(), date)?;
        let k_region = hmac_sha256(&k_date, &self.region)?;
        let k_service = hmac_sha256(&k_region, "s3")?;
        let k_signing = hmac_sha256(&k_service, "aws4_request")?;

        Ok(k_signing)
    }

    fn build_authorization(
        &self,
        method: &str,
        canonical_uri: &str,
        payload_hash: &str,
        amz_date: &str,
        date: &str,
    ) -> Result<String> {
        let endpoint_host = self
            .endpoint
            .replace("https://", "")
            .replace("http://", "");

        let canonical_headers = format!(
            "host:{}\nx-amz-content-sha256:{}\nx-amz-date:{}\n",
            endpoint_host, payload_hash, amz_date
        );

        let signed_headers = "host;x-amz-content-sha256;x-amz-date";

        let canonical_request = format!(
            "{method}\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );

        let canonical_request_hash = sha256_hex(canonical_request.as_bytes());

        let credential_scope = format!("{date}/{}/s3/aws4_request", self.region);

        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{canonical_request_hash}"
        );

        let signing_key = self.sign_key(date)?;
        let signature = hex::encode(hmac_sha256(&signing_key, &string_to_sign)?);

        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.access_key, credential_scope, signed_headers, signature
        );

        Ok(authorization)
    }
}

impl Uploader for S3Uploader {
    fn upload(&self, file_name: &str, data: &[u8]) -> Result<()> {
        let now = Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date = now.format("%Y%m%d").to_string();

        let safe_file_name = file_name.replace('\\', "/");
        let canonical_uri = format!("/{}/{}", self.bucket, safe_file_name);
        let url = format!(
            "{}/{}/{}",
            self.endpoint.trim_end_matches('/'),
            self.bucket,
            safe_file_name
        );

        let payload_hash = sha256_hex(data);

        let authorization = self.build_authorization(
            "PUT",
            &canonical_uri,
            &payload_hash,
            &amz_date,
            &date,
        )?;

        let response = self
            .client
            .put(&url)
            .header("x-amz-date", amz_date)
            .header("x-amz-content-sha256", payload_hash)
            .header("Authorization", authorization)
            .header("Content-Type", "image/png")
            .body(data.to_vec())
            .send()
            .context("Cannot upload file to S3")?;

        if !response.status().is_success() {
            anyhow::bail!("S3 upload failed with status: {}", response.status());
        }

        println!("Uploaded to S3: {file_name}");

        Ok(())
    }
}

fn hmac_sha256(key: &[u8], data: &str) -> Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key)
        .context("Cannot create HMAC")?;

    mac.update(data.as_bytes());

    Ok(mac.finalize().into_bytes().to_vec())
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn create_uploader() -> Result<Box<dyn Uploader>> {
    let uploader_type = env::var("MYME_UPLOADER")
        .unwrap_or_else(|_| "fs".to_string());

    match uploader_type.as_str() {
        "fs" => Ok(Box::new(FsUploader::new()?)),
        "s3" => Ok(Box::new(S3Uploader::new()?)),
        _ => anyhow::bail!("Unknown MYME_UPLOADER value. Use fs or s3"),
    }
}

fn parse_resize(value: &str) -> Result<(u32, u32)> {
    let parts: Vec<&str> = value.split('x').collect();

    if parts.len() != 2 {
        anyhow::bail!("Resize format must be widthxheight, for example 800x600");
    }

    let width = parts[0].parse::<u32>().context("Invalid width")?;
    let height = parts[1].parse::<u32>().context("Invalid height")?;

    Ok((width, height))
}

fn is_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn load_image(source: &str) -> Result<Vec<u8>> {
    if is_url(source) {
        let response = reqwest::blocking::get(source)
            .with_context(|| format!("Cannot download image: {source}"))?;

        let bytes = response.bytes().context("Cannot read image bytes")?;

        Ok(bytes.to_vec())
    } else {
        fs::read(source)
            .with_context(|| format!("Cannot read local image file: {source}"))
    }
}

fn create_output_name(source: &str, index: usize) -> String {
    let file_name = if is_url(source) {
        source
            .split('/')
            .last()
            .filter(|name| !name.is_empty())
            .unwrap_or("image")
            .to_string()
    } else {
        Path::new(source)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("image")
            .to_string()
    };

    format!("{index}_resized_{file_name}.png")
}

fn process_image(
    source: &str,
    width: u32,
    height: u32,
    index: usize,
    uploader: &dyn Uploader,
) -> Result<()> {
    let image_bytes = load_image(source)?;

    let image = image::load_from_memory(&image_bytes)
        .with_context(|| format!("Cannot decode image: {source}"))?;

    let resized = image.resize_exact(width, height, FilterType::Lanczos3);

    let mut buffer = Cursor::new(Vec::new());

    resized
        .write_to(&mut buffer, ImageFormat::Png)
        .context("Cannot encode resized image")?;

    let output_name = create_output_name(source, index);

    uploader.upload(&output_name, &buffer.into_inner())?;

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let uploader = create_uploader()?;

    let (width, height) = parse_resize(&cli.resize)?;

    let content = fs::read_to_string(&cli.files)
        .with_context(|| format!("Cannot read input file: {}", cli.files))?;

    for (index, line) in content.lines().enumerate() {
        let source = line.trim().trim_matches('"');

        if source.is_empty() {
            continue;
        }

        if let Err(error) = process_image(source, width, height, index + 1, uploader.as_ref()) {
            eprintln!("Error processing {source}: {error}");
        }
    }

    Ok(())
}