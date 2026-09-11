mod album;
mod config;
mod disk;
mod integrity;
mod lidarr;
mod navidrome;
mod quarantine;
mod report;
mod storage;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Debug, Parser)]
#[command(name = "media-maintenance")]
#[command(about = "Rust replacement for Lidarr/n8n maintenance workflows")]
struct Cli {
    #[arg(long)]
    env_file: Option<std::path::PathBuf>,
    #[arg(long, env = "LOG_JSON", default_value_t = false)]
    log_json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(name = "album-cleanup")]
    Album(album::AlbumArgs),
    #[command(name = "disk-cleanup")]
    Disk(disk::DiskArgs),
    #[command(name = "quarantine-cleanup")]
    Quarantine(quarantine::Args),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(path) = &cli.env_file {
        dotenvy::from_path(path)?;
    }
    init_tracing(cli.log_json);
    let _lock = if matches!(cli.command, Command::Disk(_) | Command::Quarantine(_)) {
        let dir = std::path::PathBuf::from(config::env_parse(
            "REPORT_DIR",
            String::from("/data/reports"),
        ));
        std::fs::create_dir_all(&dir)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("cleanup.lock"))?;
        fs2::FileExt::try_lock_exclusive(&file)?;
        Some(file)
    } else {
        None
    };

    match cli.command {
        Command::Album(args) => album::run(args).await,
        Command::Disk(args) => disk::run(args).await,
        Command::Quarantine(args) => quarantine::run(args).await,
    }
}

fn init_tracing(json: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if json {
        fmt().json().with_env_filter(filter).init();
    } else {
        fmt().with_env_filter(filter).init();
    }
}
