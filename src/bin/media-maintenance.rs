use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::json;

#[derive(Debug, Parser)]
#[command(name = "media-maintenance")]
#[command(about = "Dokploy scheduled maintenance CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    AlbumCleanup {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        artist_id: Option<i64>,
        #[arg(long)]
        force: bool,
    },
    DiskCleanup {
        #[arg(long)]
        dry_run: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::AlbumCleanup { dry_run, artist_id, force } => {
            println!("{}", serde_json::to_string_pretty(&json!({
                "command": "album-cleanup",
                "dryRun": dry_run,
                "artistId": artist_id,
                "force": force,
                "status": "scaffold"
            }))?);
        }
        Command::DiskCleanup { dry_run } => {
            println!("{}", serde_json::to_string_pretty(&json!({
                "command": "disk-cleanup",
                "dryRun": dry_run,
                "status": "scaffold"
            }))?);
        }
    }
    Ok(())
}
