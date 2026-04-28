#![warn(clippy::missing_errors_doc, clippy::result_large_err)]

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use clap::{Parser, Subcommand};
use crossbeam_channel::{unbounded, Receiver};
use rand::RngCore;
use rayon::prelude::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use walkdir::WalkDir;

const MATRIX_SIZE: usize = 4096;
const AES_KEY: [u8; 32] = [7; 32];

type Matrix = Vec<Vec<u64>>;

#[derive(Parser)]
#[command(name = "pz5")]
#[command(about = "Practical work 5: threads, channels and shared data")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Matrix,

    Encrypt {
        #[arg(long)]
        dir: String,
    },
}

#[derive(Debug, Clone)]
struct FileData {
    path: PathBuf,
    content: Vec<u8>,
}
fn generate_matrix() -> Matrix {
    let mut matrix = Vec::with_capacity(MATRIX_SIZE);

    for i in 0..MATRIX_SIZE {
        let mut row = Vec::with_capacity(MATRIX_SIZE);

        for j in 0..MATRIX_SIZE {
            row.push((i + j) as u64);
        }

        matrix.push(row);
    }

    matrix
}

fn calculate_matrix_sum(matrix: &Matrix) -> u64 {
    matrix
        .par_iter()
        .map(|row| row.par_iter().sum::<u64>())
        .sum()
}

fn run_matrix_task() -> Result<(), String> {
    let (sender, receiver) = unbounded::<Matrix>();

    let generator = thread::spawn(move || {
        for index in 1..=2 {
            println!("Генерується матриця №{index}...");
            let matrix = generate_matrix();

            sender
                .send(matrix)
                .map_err(|error| format!("Помилка відправлення матриці: {error}"))?;

            println!("Матриця №{index} відправлена в канал.");
        }

        Ok::<(), String>(())
    });

    let mut workers = Vec::new();

    for worker_id in 1..=2 {
        let receiver = receiver.clone();

        let worker = thread::spawn(move || {
            let matrix = receiver
                .recv()
                .map_err(|error| format!("Потік {worker_id}: помилка отримання матриці: {error}"))?;

            println!("Потік {worker_id} рахує суму...");

            let sum = calculate_matrix_sum(&matrix);

            println!("Потік {worker_id}: сума елементів = {sum}");

            Ok::<(), String>(())
        });

        workers.push(worker);
    }

    generator
        .join()
        .map_err(|_| "Помилка виконання потоку генератора".to_string())??;

    for worker in workers {
        worker
            .join()
            .map_err(|_| "Помилка виконання потоку обчислення".to_string())??;
    }

    Ok(())
}

fn encrypt_content(content: &[u8]) -> Result<Vec<u8>, String> {
    let key = Key::<Aes256Gcm>::from_slice(&AES_KEY);
    let cipher = Aes256Gcm::new(key);

    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);

    let nonce = Nonce::from_slice(&nonce_bytes);

    let encrypted = cipher
        .encrypt(nonce, content)
        .map_err(|error| format!("Помилка шифрування: {error}"))?;

    let mut result = nonce_bytes.to_vec();
    result.extend(encrypted);

    Ok(result)
}
fn producer_thread(dir: String, sender: crossbeam_channel::Sender<FileData>) -> Result<(), String> {
    for entry in WalkDir::new(dir) {
        let entry = entry.map_err(|error| format!("Помилка читання директорії: {error}"))?;

        if entry.file_type().is_file() {
            let path = entry.path().to_path_buf();

            if path.extension().is_some_and(|ext| ext == "data") {
                continue;
            }

            let content = fs::read(&path)
                .map_err(|error| format!("Помилка читання файлу {:?}: {error}", path))?;

            sender
                .send(FileData { path, content })
                .map_err(|error| format!("Помилка відправлення файлу в канал: {error}"))?;
        }
    }

    Ok(())
}

fn consumer_thread(
    id: usize,
    receiver: Receiver<FileData>,
    counter: Arc<AtomicUsize>,
) -> Result<(), String> {
    while let Ok(file_data) = receiver.recv() {
        let encrypted = encrypt_content(&file_data.content)?;

        let output_path = create_output_path(&file_data.path);

        fs::write(&output_path, encrypted)
            .map_err(|error| format!("Потік {id}: помилка запису {:?}: {error}", output_path))?;

        counter.fetch_add(1, Ordering::SeqCst);

        println!("Потік {id} зашифрував файл {:?}", file_data.path);
    }

    Ok(())
}

fn create_output_path(path: &Path) -> PathBuf {
    let mut output = path.to_path_buf();

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("encrypted");

    output.set_file_name(format!("{file_name}.data"));

    output
}

fn progress_thread(counter: Arc<AtomicUsize>, finished: Arc<AtomicBool>) {
    let mut last_value = 0;

    loop {
        let current_value = counter.load(Ordering::SeqCst);

        if current_value != last_value {
            println!("Оброблено файлів: {current_value}");
            last_value = current_value;
        }

        if finished.load(Ordering::SeqCst) {
            let final_value = counter.load(Ordering::SeqCst);

            if final_value != last_value {
                println!("Оброблено файлів: {final_value}");
            }

            break;
        }

        thread::sleep(Duration::from_millis(300));
    }
}

fn run_encrypt_task(dir: String) -> Result<(), String> {
    let (sender, receiver) = unbounded::<FileData>();

    let counter = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicBool::new(false));

    let producer = {
        let sender = sender.clone();

        thread::spawn(move || producer_thread(dir, sender))
    };

    drop(sender);

    let mut consumers = Vec::new();

    for id in 1..=3 {
        let receiver = receiver.clone();
        let counter = Arc::clone(&counter);

        let consumer = thread::spawn(move || consumer_thread(id, receiver, counter));

        consumers.push(consumer);
    }

    let progress = {
        let counter = Arc::clone(&counter);
        let finished = Arc::clone(&finished);

        thread::spawn(move || progress_thread(counter, finished))
    };

    producer
        .join()
        .map_err(|_| "Помилка виконання producer потоку".to_string())??;

    drop(receiver);

    for consumer in consumers {
        consumer
            .join()
            .map_err(|_| "Помилка виконання consumer потоку".to_string())??;
    }

    finished.store(true, Ordering::SeqCst);

    progress
        .join()
        .map_err(|_| "Помилка виконання progress потоку".to_string())?;

    println!("Шифрування завершено.");

    Ok(())
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Matrix => run_matrix_task(),
        Commands::Encrypt { dir } => run_encrypt_task(dir),
    };

    if let Err(error) = result {
        eprintln!("Помилка: {error}");
    }
}