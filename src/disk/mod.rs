use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{anyhow, Result};
use clap::Args;
use futures::{stream, StreamExt};
use serde::{Deserialize, Serialize};
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
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct FsFile {
    source_path: PathBuf,
    lidarr_path: String,
    size: u64,
    mtime_ms: i64,
    mtime_ns: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    #[serde(default)]
    mtime_ns: u128,
    #[serde(default)]
    quarantined_at: Option<String>,
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
    navidrome_response: Option<Value>,
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

    let evidence = crate::integrity::fetch().await?;
    let artists = lidarr.artists().await?;
    let albums = lidarr.all_albums().await?;
    let track_files = fetch_track_files(&lidarr, &artists, cfg.scan_concurrency).await?;
    let manual_import = if env_parse("DISK_CLEAN_UNTRACKED", false) {
        lidarr.manual_import(&cfg.lidarr_music_root).await?
    } else {
        Vec::new()
    };
    let files = scan_files(&cfg).await?;
    let filesystem_files = files.len();
    let mut records = classify(
        &cfg,
        files,
        &artists,
        &albums,
        &track_files,
        &manual_import,
        &evidence,
    )?;
    records.sort_by(|a, b| a.source_path.cmp(&b.source_path));
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
        Some(match lidarr.rescan_folders(&affected_artist_paths).await {
            Ok(v) => v,
            Err(e) => json!({"error": e.to_string()}),
        })
    } else {
        None
    };

    let moved = records.iter().any(|r| r.action == "quarantined");
    let navidrome_response = if moved && std::env::var("NAVIDROME_URL").is_ok() {
        Some(match crate::navidrome::rescan().await {
            Ok(v) => v,
            Err(e) => json!({"error": e.to_string()}),
        })
    } else {
        None
    };
    let failed = records.iter().any(|r| r.action == "move_failed")
        || rescan_response
            .as_ref()
            .is_some_and(|r| r.get("error").is_some())
        || navidrome_response
            .as_ref()
            .is_some_and(|r| r.get("error").is_some());
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
        scanned: json!({ "filesystem_files": filesystem_files, "lidarr_track_files": track_files.len(), "manual_import_items": manual_import.len() }),
        records,
        rescan_response,
        navidrome_response,
    };

    fs::write(&manifest_path, serde_json::to_vec_pretty(&report)?).await?;
    let _ = write_json_report(&app.report_dir, "disk-cleanup", &run_id, &report).await?;
    println!(
        "{}",
        serde_json::to_string(
            &json!({"run_id":report.run_id,"mode":report.mode,"counts":report.counts,"manifest_path":report.manifest_path,"rescan_required":report.rescan_required})
        )?
    );
    if failed {
        return Err(anyhow!("Cleanup or rescan failed; see saved report"));
    }
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
                mtime_ns: md.modified()?.duration_since(UNIX_EPOCH)?.as_nanos(),
                mtime_ms,
            });
        }
        Result::<Vec<FsFile>>::Ok(files)
    })
    .await?
}

#[allow(clippy::too_many_arguments)]
fn classify(
    cfg: &DiskConfig,
    files: Vec<FsFile>,
    artists: &[Value],
    albums: &[Value],
    track_files: &[Value],
    manual_import: &[Value],
    evidence: &HashMap<String, crate::integrity::Evidence>,
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
                .map(|p| (normalize_path(p), f))
        })
        .collect::<HashMap<_, _>>();
    let manual_by_path = manual_import
        .iter()
        .filter_map(|m| {
            m.get("path")
                .and_then(Value::as_str)
                .map(|p| (normalize_path(p), m))
        })
        .collect::<HashMap<_, _>>();
    let mut records = Vec::new();

    for file in files {
        let key = file.lidarr_path.clone();
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
        let integrity_root = PathBuf::from(env_parse(
            "AUDIO_INTEGRITY_MUSIC_ROOT",
            String::from("/music"),
        ));
        let integrity_path = integrity_root.join(file.source_path.strip_prefix(&cfg.music_root)?);
        if let Some(item) = evidence.get(&integrity_path.to_string_lossy().to_string()) {
            if item.matches(&file.source_path, file.size, file.mtime_ns) {
                if let Some(category) = item.category() {
                    let active = active_by_path.get(&key);
                    let artist_id = active
                        .and_then(|a| a.get("artistId"))
                        .and_then(Value::as_i64);
                    let artist_path = artist_id
                        .and_then(|x| artists_by_id.get(&x))
                        .and_then(|a| a.get("path"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    records.push(record(
                        cfg,
                        &file,
                        category,
                        &format!("{}; {}", item.message, item.authenticity_message),
                        artist_id,
                        active
                            .and_then(|a| a.get("albumId"))
                            .and_then(Value::as_i64),
                        active.and_then(|a| id(a)),
                        active.is_some(),
                        artist_path,
                    )?);
                    continue;
                }
            }
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
        mtime_ns: file.mtime_ns,
        quarantined_at: None,
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
            "known_unmonitored_album"
                | "unknown_to_lidarr"
                | "duplicate_lower_quality"
                | "audio_corrupt"
                | "audio_likely_lossy"
        ) {
            continue;
        }
        if matches!(
            rec.category.as_str(),
            "unknown_to_lidarr" | "duplicate_lower_quality"
        ) && !env_parse("DISK_CLEAN_UNTRACKED", false)
        {
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

    if !selected.is_empty() {
        let journal_dir = cfg.quarantine_root.join(run_id);
        fs::create_dir_all(&journal_dir).await?;
        let journal = journal_dir.join("moves.json");
        let created_at = now_iso();
        for &idx in &selected {
            records[idx].action = "pending".into();
        }
        save_moves(&journal, &created_at, records).await?;
        for idx in selected {
            let rec = &mut records[idx];
            let outcome: Result<()> = async {
                let md = fs::symlink_metadata(&rec.source_path).await?;
                if !md.is_file()
                    || md.len() != rec.size
                    || md.modified()?.duration_since(UNIX_EPOCH)?.as_nanos() != rec.mtime_ns
                {
                    return Err(anyhow!("file changed since classification"));
                }
                if rec.quarantine_path.exists() {
                    return Err(anyhow!("quarantine destination exists"));
                }
                let source = fs::canonicalize(&rec.source_path).await?;
                if !source.starts_with(fs::canonicalize(&cfg.music_root).await?) {
                    return Err(anyhow!("source outside music root"));
                }
                if let Some(parent) = rec.quarantine_path.parent() {
                    fs::create_dir_all(parent).await?;
                }
                fs::rename(&rec.source_path, &rec.quarantine_path).await?;
                Ok(())
            }
            .await;
            match outcome {
                Ok(()) => {
                    rec.action = "quarantined".into();
                    rec.quarantined_at = Some(now_iso());
                }
                Err(e) => {
                    rec.action = "move_failed".into();
                    rec.reason = format!("{}; {e}", rec.reason);
                }
            }
            save_moves(&journal, &created_at, records).await?;
        }
    }
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

async fn save_moves(path: &Path, created_at: &str, records: &[Record]) -> Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&json!({"created_at":created_at,"records":records}))?,
    )
    .await?;
    fs::rename(temporary, path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (DiskConfig, FsFile) {
        let root =
            std::env::temp_dir().join(format!("media-maintenance-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("music")).unwrap();
        let path = root.join("music/track.flac");
        std::fs::write(&path, b"fixture").unwrap();
        let md = path.metadata().unwrap();
        let cfg = DiskConfig {
            dry_run: false,
            music_root: root.join("music"),
            lidarr_music_root: "/music".into(),
            quarantine_root: root.join("quarantine"),
            stale_hours: 0,
            max_files: 10,
            max_bytes: 100,
            scan_concurrency: 1,
        };
        let file = FsFile {
            source_path: path,
            lidarr_path: "/music/track.flac".into(),
            size: md.len(),
            mtime_ms: 0,
            mtime_ns: md
                .modified()
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        };
        (cfg, file)
    }

    #[tokio::test]
    async fn quarantine_records_success_and_preserves_changed_files() {
        let (cfg, file) = fixture();
        let mut records = vec![record(
            &cfg,
            &file,
            "audio_corrupt",
            "decode failure",
            None,
            None,
            None,
            false,
            None,
        )
        .unwrap()];
        std::fs::write(&file.source_path, b"replacement content").unwrap();
        apply_limits(&cfg, "disk-cleanup-test", &mut records)
            .await
            .unwrap();
        assert_eq!(records[0].action, "move_failed");
        assert!(file.source_path.exists());
        let md = file.source_path.metadata().unwrap();
        records[0].size = md.len();
        records[0].mtime_ns = md
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        apply_limits(&cfg, "disk-cleanup-test2", &mut records)
            .await
            .unwrap();
        assert_eq!(records[0].action, "quarantined");
        assert!(records[0].quarantined_at.is_some());
        assert!(!file.source_path.exists());
        let journal: Value = serde_json::from_slice(
            &std::fs::read(cfg.quarantine_root.join("disk-cleanup-test2/moves.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(journal["records"][0]["action"], "quarantined");
        std::fs::remove_dir_all(cfg.music_root.parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn dry_run_does_not_move_and_limits_are_enforced() {
        let (mut cfg, file) = fixture();
        cfg.dry_run = true;
        let rec = record(
            &cfg,
            &file,
            "audio_likely_lossy",
            "heuristic",
            None,
            None,
            None,
            false,
            None,
        )
        .unwrap();
        let mut records = vec![rec.clone(), rec];
        cfg.max_files = 1;
        apply_limits(&cfg, "disk-cleanup-test", &mut records)
            .await
            .unwrap();
        assert_eq!(records[0].action, "dry_run");
        assert_eq!(records[1].action, "skipped_limit");
        assert!(file.source_path.exists());
        assert!(!cfg.quarantine_root.exists());
        std::fs::remove_dir_all(cfg.music_root.parent().unwrap()).unwrap();
    }

    #[test]
    fn integrity_requires_matching_fingerprint_and_protects_recent_files() {
        let (mut cfg, file) = fixture();
        let mut evidence = HashMap::new();
        evidence.insert(
            file.lidarr_path.clone(),
            crate::integrity::Evidence {
                path: file.lidarr_path.clone(),
                size: file.size,
                mtime_ns: file.mtime_ns.to_string(),
                verdict: "corrupt".into(),
                authenticity: "unknown".into(),
                message: "decode failure".into(),
                authenticity_message: "".into(),
                validator_version: "test".into(),
            },
        );
        let classify_file =
            |cfg: &DiskConfig, f: FsFile, e: &HashMap<String, crate::integrity::Evidence>| {
                classify(cfg, vec![f], &[], &[], &[], &[], e)
                    .unwrap()
                    .remove(0)
            };
        assert_eq!(
            classify_file(&cfg, file.clone(), &evidence).category,
            "audio_corrupt"
        );
        evidence.get_mut(&file.lidarr_path).unwrap().mtime_ns = "1".into();
        assert_eq!(
            classify_file(&cfg, file.clone(), &evidence).category,
            "unknown_to_lidarr"
        );
        cfg.stale_hours = 72;
        let mut recent = file.clone();
        recent.mtime_ms = chrono::Utc::now().timestamp_millis();
        assert_eq!(
            classify_file(&cfg, recent, &evidence).category,
            "report_only"
        );
        std::fs::remove_dir_all(cfg.music_root.parent().unwrap()).unwrap();
    }
}
