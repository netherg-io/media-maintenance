use anyhow::Result;
use clap::Args;
use serde::Serialize;
use serde_json::json;
use walkdir::WalkDir;

use crate::{
    config::{env_parse, AppConfig},
    lidarr::Lidarr,
    report::{now_iso, run_id, write_json_report},
};

#[derive(Debug, Clone, Args)]
pub struct DiskArgs {
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long, env = "DISK_SCAN_CONCURRENCY")]
    pub scan_concurrency: Option<usize>,
    #[arg(long, env = "DISK_MOVE_CONCURRENCY")]
    pub move_concurrency: Option<usize>,
}

#[derive(Debug, Serialize)]
struct DiskReport {
    workflow: &'static str,
    run_id: String,
    mode: String,
    started_at: String,
    finished_at: String,
    filesystem_audio_files: usize,
    lidarr_artists: usize,
    lidarr_albums: usize,
    lidarr_manual_import_items: usize,
    note: String,
}

pub async fn run(args: DiskArgs) -> Result<()> {
    let started_at = now_iso();
    let run_id = run_id("disk-cleanup");
    let app = AppConfig::from_env()?;
    let dry_run = args.dry_run || env_parse("DISK_DRY_RUN", true);
    let music_root = std::path::PathBuf::from(env_parse("DISK_MUSIC_ROOT", String::from("/media/music")));
    let lidarr_music_root = env_parse("DISK_LIDARR_MUSIC_ROOT", String::from("/music"));
    let concurrency = args.scan_concurrency.unwrap_or_else(|| env_parse("DISK_SCAN_CONCURRENCY", 16));
    let lidarr = Lidarr::new(app.lidarr_base_url.clone(), app.lidarr_header_value.clone(), concurrency);

    let artists = lidarr.artists().await.unwrap_or_default();
    let albums = lidarr.all_albums().await.unwrap_or_default();
    let manual_import = lidarr.manual_import(&lidarr_music_root).await.unwrap_or_default();
    let filesystem_audio_files = if music_root.exists() { count_audio_files(&music_root) } else { 0 };

    let report = DiskReport {
        workflow: "Disk cleanup",
        run_id: run_id.clone(),
        mode: if dry_run { "dry-run" } else { "apply" }.into(),
        started_at,
        finished_at: now_iso(),
        filesystem_audio_files,
        lidarr_artists: artists.len(),
        lidarr_albums: albums.len(),
        lidarr_manual_import_items: manual_import.len(),
        note: "Initial Rust port scaffold: reports scan scope; quarantine classifier can be extended safely after first CI run.".into(),
    };

    let _ = write_json_report(&app.report_dir, "disk-cleanup", &run_id, &report).await?;
    println!("{}", serde_json::to_string_pretty(&json!(report))?);
    Ok(())
}

fn count_audio_files(root: &std::path::Path) -> usize {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            matches!(
                e.path().extension().and_then(|x| x.to_str()).unwrap_or_default().to_lowercase().as_str(),
                "mp3" | "flac" | "m4a" | "aac" | "ogg" | "opus" | "wav" | "aiff" | "alac" | "ape"
            )
        })
        .count()
}
