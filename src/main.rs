mod album;
mod config;
mod disk;
mod lidarr;
mod report;
mod storage;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Debug, Parser)]
#[command(name = "media-maintenance")]
#[command(about = "Rust replacement for Lidarr/n8n maintenance workflows")]
struct Cli {
    #[arg(long, env = "LOG_JSON", default_value_t = false)]
    log_json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    AlbumCleanup(album::AlbumArgs),
    DiskCleanup(disk::DiskArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.log_json);

    match cli.command {
        Command::AlbumCleanup(args) => album::run(args).await,
        Command::DiskCleanup(args) => disk::run(args).await,
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
