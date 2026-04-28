#![warn(clippy::missing_errors_doc, clippy::result_large_err)]

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use filesindex_core::{
    parse_tags, FileRecord, IndexStorage, JsonStorage, SqliteStorage,
};
use std::env;

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

fn create_storage() -> Result<Box<dyn IndexStorage>> {
    let config = env::var("FILES_INDEX_PATH")
        .context("Environment variable FILES_INDEX_PATH is not set")?;

    let Some((storage_type, path)) = config.split_once(':') else {
        bail!("Invalid FILES_INDEX_PATH format. Example: json:index.json");
    };

    match storage_type {
        "json" => Ok(Box::new(JsonStorage::new(path.to_string()))),
        "sqlite" => Ok(Box::new(SqliteStorage::new(path.to_string()))),
        _ => bail!("Unknown storage type. Use json or sqlite"),
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    let mut storage = create_storage()?;

    match cli.command {
        Commands::Add { path, tags } => {
            let parsed_tags = parse_tags(&tags);

            let record = FileRecord {
                path,
                tags: parsed_tags,
            };

            storage
                .add_file(record)
                .context("Failed to add file to index")?;

            println!("Файл успішно додано до індексу.");
        }

        Commands::Get { tags } => {
            let parsed_tags = parse_tags(&tags);

            let records = storage
                .get_files_by_tags(parsed_tags)
                .context("Failed to get files by tags")?;

            if records.is_empty() {
                println!("Файли з такими тегами не знайдено.");
            } else {
                println!("Знайдені файли:");

                for record in records {
                    println!("Файл: {}", record.path);
                    println!("Теги: {}", record.tags.join(", "));
                    println!("------------------------");
                }
            }
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    run()
}