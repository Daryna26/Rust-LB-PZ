use anyhow::{Context, Result};
use clap::Parser;
use image::imageops::FilterType;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "image_editor")]
#[command(about = "CLI program for resizing images from file paths or URLs")]
struct Cli {
    #[arg(long)]
    files: String,

    #[arg(long)]
    resize: String,
}

fn parse_resize(value: &str) -> Result<(u32, u32)> {
    let parts: Vec<&str> = value.split('x').collect();

    if parts.len() != 2 {
        anyhow::bail!("Resize format must be widthxheight, for example 800x600");
    }

    let width = parts[0]
        .parse::<u32>()
        .context("Invalid width value")?;

    let height = parts[1]
        .parse::<u32>()
        .context("Invalid height value")?;

    Ok((width, height))
}

fn is_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn load_image(source: &str) -> Result<Vec<u8>> {
    if is_url(source) {
        let response = reqwest::blocking::get(source)
            .with_context(|| format!("Cannot download image: {source}"))?;

        let bytes = response
            .bytes()
            .context("Cannot read response bytes")?;

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

    format!("{index}_resized_{file_name}")
}

fn process_image(source: &str, output_dir: &Path, width: u32, height: u32, index: usize) -> Result<()> {
    let image_bytes = load_image(source)?;

    let image = image::load_from_memory(&image_bytes)
        .with_context(|| format!("Cannot decode image: {source}"))?;

    let resized = image.resize_exact(width, height, FilterType::Lanczos3);

    let output_name = create_output_name(source, index);
    let output_path = output_dir.join(output_name);

    resized
        .save(&output_path)
        .with_context(|| format!("Cannot save image to {}", output_path.display()))?;

    println!("Saved: {}", output_path.display());

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let output_dir = env::var("MYME_FILES_PATH")
        .context("Environment variable MYME_FILES_PATH is not set")?;

    let output_dir = PathBuf::from(output_dir);

    if !output_dir.exists() {
        fs::create_dir_all(&output_dir)
            .context("Cannot create output directory")?;
    }

    let (width, height) = parse_resize(&cli.resize)?;

    let content = fs::read_to_string(&cli.files)
        .with_context(|| format!("Cannot read input file: {}", cli.files))?;

    for (index, line) in content.lines().enumerate() {
        let source = line.trim();

        if source.is_empty() {
            continue;
        }

        if let Err(error) = process_image(source, &output_dir, width, height, index + 1) {
            eprintln!("Error processing {source}: {error}");
        }
    }

    Ok(())
}