use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{anyhow, Result};
use clap::Args;
use futures::{stream, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::fs;
use walkdir::WalkDir;

use crate::{
    config::{env_parse, AppConfig},
    lidarr::{bool_field, id, Lidarr},
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

#[derive(Debug, Clone)]
struct DiskConfig {
    dry_run: bool,
    music_root: PathBuf,
    lidarr_music_root: String,
    quarantine_root: PathBuf,
    stale_hours: i64,
    max_files: usize,
    max_bytes: u64,
    scan_concurrency: usize,
    move_concurrency: usize,
}

impl DiskConfig {
    fn from_env(args: &DiskArgs) -> Self {
        Self {
            dry_run: args.dry_run || env_parse("DISK_DRY_RUN", true),
            music_root: PathBuf::from(env_parse("DISK_MUSIC_ROOT", String::from("/media/music"))),
            lidarr_music_root: env_parse("DISK_LIDARR_MUSIC_ROOT", String::from("/music")),
            quarantine_root: PathBuf::from(env_parse(
                "DISK_QUARANTINE_ROOT",
                String::from("/media/.cleanup-quarantine"),
            )),
            stale_hours: env_parse("DISK_STALE_HOURS", 72),
            max_files: env_parse("DISK_MAX_FILES", 100),
            max_bytes: env_parse("DISK_MAX_BYTES", 26_843_545_600u64),
            scan_concurrency: args
                .scan_concurrency
                .unwrap_or_else(|| env_parse("DISK_SCAN_CONCURRENCY", 16))
                .max(1),
            move_concurrency: args
                .move_concurrency
                .unwrap_or_else(|| env_parse("DISK_MOVE_CONCURRENCY", 2))
                .max(1),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct FsFile {
    source_path: PathBuf,
    lidarr_path: String,
    size: u64,
    mtime_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
struct Record {
    category: String,
    reason: String,
    source_path: PathBuf,
    lidarr_path: String,
    quarantine_path: PathBuf,
    size: u64,
    artist_id: Option<i64>,
    album_id: Option<i64>,
    track_file_id: Option<i64>,
    action: String,
    selected_for_move: bool,
    active_known: bool,
    artist_path: Option<String>,
}

#[derive(Debug, Default, Serialize)]
struct Counts {
    by_category: HashMap<String, usize>,
    by_action: HashMap<String, usize>,
}

#[derive(Debug, Serialize)]
struct DiskReport {
    workflow: &'static str,
    run_id: String,
    mode: String,
    started_at: String,
    finished_at: String,
    manifest_path: String,
    rescan_required: bool,
    affected_artist_paths: Vec<String>,
    counts: Counts,
    scanned: Value,
    records: Vec<Record>,
    rescan_response: Option<Value>,
}

pub async fn run(args: DiskArgs) -> Result<()> {
    let started_at = now_iso();
    let run_id = run_id("disk-cleanup");
    let app = AppConfig::from_env()?;
    let cfg = DiskConfig::from_env(&args);
    let lidarr = Lidarr::new(
        app.lidarr_base_url.clone(),
        app.lidarr_header_value.clone(),
        cfg.scan_concurrency,
    );

    if !cfg.music_root.exists() {
        return Err(anyhow!(
            "music root does not exist: {}",
            cfg.music_root.display()
        ));
    }

    let artists = lidarr.artists().await?;
    let albums = lidarr.all_albums().await?;
    let track_files = fetch_track_files(&lidarr, &artists, cfg.scan_concurrency).await?;
    let manual_import = lidarr
        .manual_import(&cfg.lidarr_music_root)
        .await
        .unwrap_or_default();
    let files = scan_files(&cfg).await?;
    let mut records = classify(&cfg, files, &artists, &albums, &track_files, &manual_import)?;
    apply_limits(&cfg, &run_id, &mut records).await?;

    let affected_artist_paths = records
        .iter()
        .filter(|r| r.action == "quarantined" && r.active_known)
        .filter_map(|r| r.artist_path.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let rescan_required = !cfg.dry_run && !affected_artist_paths.is_empty();
    let rescan_response = if rescan_required {
        Some(lidarr.rescan_folders(&affected_artist_paths).await?)
    } else {
        None
    };

    let counts = count(&records);
    let manifest_dir = cfg.quarantine_root.join(&run_id);
    fs::create_dir_all(&manifest_dir).await?;
    let manifest_path = manifest_dir.join("manifest.json");

    let report = DiskReport {
        workflow: "Disk cleanup",
        run_id: run_id.clone(),
        mode: if cfg.dry_run { "dry-run" } else { "apply" }.into(),
        started_at,
        finished_at: now_iso(),
        manifest_path: manifest_path.display().to_string(),
        rescan_required,
        affected_artist_paths,
        counts,
        scanned: json!({ "filesystem_files": records.len(), "lidarr_track_files": track_files.len(), "manual_import_items": manual_import.len() }),
        records,
        rescan_response,
    };

    fs::write(&manifest_path, serde_json::to_vec_pretty(&report)?).await?;
    let _ = write_json_report(&app.report_dir, "disk-cleanup", &run_id, &report).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn fetch_track_files(
    lidarr: &Lidarr,
    artists: &[Value],
    concurrency: usize,
) -> Result<Vec<Value>> {
    let ids = artists.iter().filter_map(id).collect::<Vec<_>>();
    let batches = stream::iter(ids.into_iter().map(|artist_id| {
        let lidarr = lidarr.clone();
        async move { lidarr.track_files_for_artist(artist_id).await }
    }))
    .buffer_unordered(concurrency)
    .collect::<Vec<_>>()
    .await;
    let mut out = Vec::new();
    for batch in batches {
        out.extend(batch?);
    }
    Ok(out)
}

async fn scan_files(cfg: &DiskConfig) -> Result<Vec<FsFile>> {
    let root = cfg.music_root.clone();
    let quarantine = cfg.quarantine_root.clone();
    let lidarr_root = cfg.lidarr_music_root.clone();
    tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();
        for entry in WalkDir::new(&root).follow_links(false) {
            let entry = entry?;
            if !entry.file_type().is_file() || entry.path().starts_with(&quarantine) {
                continue;
            }
            let ext = entry
                .path()
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if !matches!(
                ext.as_str(),
                "mp3" | "flac" | "m4a" | "aac" | "ogg" | "opus" | "wav" | "aiff" | "alac" | "ape"
            ) {
                continue;
            }
            let md = entry.metadata()?;
            let rel = entry
                .path()
                .strip_prefix(&root)?
                .to_string_lossy()
                .replace('\\', "/");
            let mtime_ms = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or_default();
            files.push(FsFile {
                source_path: entry.path().to_path_buf(),
                lidarr_path: normalize_path(&format!(
                    "{}/{}",
                    lidarr_root.trim_end_matches('/'),
                    rel
                )),
                size: md.len(),
                mtime_ms,
            });
        }
        Result::<Vec<FsFile>>::Ok(files)
    })
    .await?
}

fn classify(
    cfg: &DiskConfig,
    files: Vec<FsFile>,
    artists: &[Value],
    albums: &[Value],
    track_files: &[Value],
    manual_import: &[Value],
) -> Result<Vec<Record>> {
    let stale_before = chrono::Utc::now().timestamp_millis() - cfg.stale_hours * 60 * 60 * 1000;
    let albums_by_id = albums
        .iter()
        .filter_map(|a| id(a).map(|x| (x, a)))
        .collect::<HashMap<_, _>>();
    let artists_by_id = artists
        .iter()
        .filter_map(|a| id(a).map(|x| (x, a)))
        .collect::<HashMap<_, _>>();
    let active_by_path = track_files
        .iter()
        .filter_map(|f| {
            f.get("path")
                .and_then(Value::as_str)
                .map(|p| (normalize_path(p).to_lowercase(), f))
        })
        .collect::<HashMap<_, _>>();
    let manual_by_path = manual_import
        .iter()
        .filter_map(|m| {
            m.get("path")
                .and_then(Value::as_str)
                .map(|p| (normalize_path(p).to_lowercase(), m))
        })
        .collect::<HashMap<_, _>>();
    let mut records = Vec::new();

    for file in files {
        let key = file.lidarr_path.to_lowercase();
        if file.mtime_ms > stale_before {
            records.push(record(
                cfg,
                &file,
                "report_only",
                "file_newer_than_stale_hours",
                None,
                None,
                None,
                false,
                None,
            )?);
            continue;
        }
        if let Some(active) = active_by_path.get(&key) {
            let album_id = active.get("albumId").and_then(Value::as_i64);
            let artist_id = active.get("artistId").and_then(Value::as_i64);
            let album_unmonitored = album_id
                .and_then(|x| albums_by_id.get(&x))
                .and_then(|a| bool_field(a, "monitored"))
                .map(|x| !x)
                .unwrap_or(false);
            if album_unmonitored {
                let artist_path = artist_id
                    .and_then(|x| artists_by_id.get(&x))
                    .and_then(|a| a.get("path"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                records.push(record(
                    cfg,
                    &file,
                    "known_unmonitored_album",
                    "active_lidarr_file_on_unmonitored_album",
                    artist_id,
                    album_id,
                    id(active),
                    true,
                    artist_path,
                )?);
            }
            continue;
        }
        if manual_by_path.contains_key(&key) {
            records.push(record(
                cfg,
                &file,
                "duplicate_lower_quality",
                "manual_import_mapping_exists_but_file_is_not_active",
                None,
                None,
                None,
                false,
                None,
            )?);
        } else {
            records.push(record(
                cfg,
                &file,
                "unknown_to_lidarr",
                "not_active_and_not_seen_by_manual_import",
                None,
                None,
                None,
                false,
                None,
            )?);
        }
    }
    Ok(records)
}

#[allow(clippy::too_many_arguments)]
fn record(
    cfg: &DiskConfig,
    file: &FsFile,
    category: &str,
    reason: &str,
    artist_id: Option<i64>,
    album_id: Option<i64>,
    track_file_id: Option<i64>,
    active_known: bool,
    artist_path: Option<String>,
) -> Result<Record> {
    Ok(Record {
        category: category.into(),
        reason: reason.into(),
        source_path: file.source_path.clone(),
        lidarr_path: file.lidarr_path.clone(),
        quarantine_path: destination(&cfg.music_root, &cfg.quarantine_root, &file.source_path)?,
        size: file.size,
        artist_id,
        album_id,
        track_file_id,
        action: "report_only".into(),
        selected_for_move: false,
        active_known,
        artist_path,
    })
}

async fn apply_limits(cfg: &DiskConfig, run_id: &str, records: &mut [Record]) -> Result<()> {
    let mut count = 0usize;
    let mut bytes = 0u64;
    let mut selected = Vec::new();
    for (idx, rec) in records.iter_mut().enumerate() {
        if !matches!(
            rec.category.as_str(),
            "known_unmonitored_album" | "unknown_to_lidarr" | "duplicate_lower_quality"
        ) {
            continue;
        }
        if count >= cfg.max_files || bytes.saturating_add(rec.size) > cfg.max_bytes {
            rec.action = "skipped_limit".into();
            continue;
        }
        count += 1;
        bytes = bytes.saturating_add(rec.size);
        rec.quarantine_path = destination(
            &cfg.music_root,
            &cfg.quarantine_root.join(run_id),
            &rec.source_path,
        )?;
        rec.action = if cfg.dry_run {
            "dry_run"
        } else {
            "quarantined"
        }
        .into();
        rec.selected_for_move = !cfg.dry_run;
        if !cfg.dry_run {
            selected.push(idx);
        }
    }

    stream::iter(selected.into_iter().map(|idx| {
        let src = records[idx].source_path.clone();
        let dst = records[idx].quarantine_path.clone();
        async move {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).await?;
            }
            fs::rename(&src, &dst).await?;
            Result::<()>::Ok(())
        }
    }))
    .buffer_unordered(cfg.move_concurrency)
    .collect::<Vec<_>>()
    .await
    .into_iter()
    .collect::<Result<Vec<_>>>()?;
    Ok(())
}

fn destination(root: &Path, quarantine: &Path, source: &Path) -> Result<PathBuf> {
    let rel = source.strip_prefix(root)?;
    Ok(quarantine.join(rel))
}

fn count(records: &[Record]) -> Counts {
    let mut counts = Counts::default();
    for r in records {
        *counts.by_category.entry(r.category.clone()).or_default() += 1;
        *counts.by_action.entry(r.action.clone()).or_default() += 1;
    }
    counts
}

fn normalize_path(value: &str) -> String {
    let mut out = value.replace('\\', "/");
    if !out.starts_with('/') {
        out.insert(0, '/');
    }
    out
}
