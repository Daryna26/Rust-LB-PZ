#![warn(clippy::missing_errors_doc, clippy::result_large_err)]

use anyhow::{Context, Result};
use clap::Parser;
use futures::stream::{self, StreamExt};
use reqwest::Client;
use std::io::{self, Read};
use std::path::PathBuf;
use tokio::fs;
use tokio::runtime::Builder;
use url::Url;

#[derive(Parser, Debug)]
#[command(name = "web-downloader")]
#[command(about = "Async web downloader with futures and tokio refactoring")]
struct Cli {
    #[arg(long)]
    max_threads: Option<usize>,

    file: Option<PathBuf>,
}

fn read_links(file: Option<PathBuf>) -> Result<Vec<String>> {
    let content = match file {
        Some(path) => std::fs::read_to_string(&path)
            .with_context(|| format!("Cannot read file: {}", path.display()))?,
        None => {
            let mut buffer = String::new();

            io::stdin()
                .read_to_string(&mut buffer)
                .context("Cannot read links from stdin")?;

            buffer
        }
    };

    Ok(content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect())
}

fn create_file_name(url: &str, index: usize) -> String {
    if let Ok(parsed_url) = Url::parse(url) {
        let host = parsed_url.host_str().unwrap_or("page");

        let mut path = parsed_url.path().replace('/', "_");

        if path.is_empty() || path == "_" {
            path = "index".to_string();
        }

        let mut name = format!("{index}_{host}{path}.html");

        name = name
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
                    ch
                } else {
                    '_'
                }
            })
            .collect();

        return name;
    }

    format!("{index}_page.html")
}


async fn download_page(client: Client, url: String, index: usize) -> Result<()> {
    println!("Downloading: {url}");

    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("Request failed: {url}"))?
        .error_for_status()
        .with_context(|| format!("Bad HTTP status: {url}"))?;

    let body = response
        .text()
        .await
        .with_context(|| format!("Cannot read response body: {url}"))?;

    let file_name = create_file_name(&url, index);

    fs::write(&file_name, body)
        .await
        .with_context(|| format!("Cannot write file: {file_name}"))?;

    println!("Saved: {file_name}");

    Ok(())
}

async fn run_downloader(links: Vec<String>, max_concurrent: usize) -> Result<()> {
    if links.is_empty() {
        println!("Список посилань порожній.");
        return Ok(());
    }

    let client = Client::new();

    stream::iter(links.into_iter().enumerate())
        .map(|(index, url)| {
            let client = client.clone();

            async move {
                download_page(client, url, index + 1).await
            }
        })
        .buffer_unordered(max_concurrent)
        .for_each(|result| async {
            if let Err(error) = result {
                eprintln!("Помилка: {error:#}");
            }
        })
        .await;

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let threads = cli.max_threads.unwrap_or_else(num_cpus::get);
    let links = read_links(cli.file)?;

    let runtime = Builder::new_multi_thread()
        .worker_threads(threads)
        .enable_all()
        .build()
        .context("Cannot build Tokio runtime")?;

    runtime.block_on(run_downloader(links, threads))?;

    Ok(())
}