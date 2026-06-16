use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use clap::Args;
use futures::{stream, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    config::{env_parse, AppConfig},
    lidarr::{
        album_title, album_type, artist_name, bool_field, duration_ms, id, track_album_id,
        track_title, Lidarr,
    },
    report::{now_iso, run_id, write_json_report},
    storage::{ArtistCache, Store},
};

#[derive(Debug, Clone, Args)]
pub struct AlbumArgs {
    #[arg(long)]
    pub artist_id: Option<i64>,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long, env = "ALBUM_CONCURRENCY")]
    pub concurrency: Option<usize>,
}

#[derive(Debug, Clone)]
struct AlbumConfig {
    target_metadata_profile_id: i64,
    apply: bool,
    concurrency: usize,
    max_artists_per_run: usize,
    cache_days: i64,
    duration_tolerance_ms: i64,
    only_monitored_artists: bool,
    only_monitored_releases: bool,
    auto_unmonitor_duplicates: bool,
}

impl AlbumConfig {
    fn from_env(args: &AlbumArgs) -> Self {
        Self {
            target_metadata_profile_id: env_parse("ALBUM_TARGET_METADATA_PROFILE_ID", 3),
            apply: env_parse("ALBUM_APPLY", true) && !args.dry_run,
            concurrency: args
                .concurrency
                .unwrap_or_else(|| env_parse("ALBUM_CONCURRENCY", 16))
                .max(1),
            max_artists_per_run: env_parse("ALBUM_MAX_ARTISTS_PER_RUN", 1000),
            cache_days: env_parse("ALBUM_CACHE_DAYS", 30),
            duration_tolerance_ms: env_parse("ALBUM_DURATION_TOLERANCE_MS", 5000),
            only_monitored_artists: env_parse("ALBUM_PROCESS_ONLY_MONITORED_ARTISTS", true),
            only_monitored_releases: env_parse("ALBUM_PROCESS_ONLY_MONITORED_RELEASES", true),
            auto_unmonitor_duplicates: env_parse("ALBUM_AUTO_UNMONITOR_DUPLICATES", true),
        }
    }
}

#[derive(Debug, Default, Serialize)]
struct Summary {
    artists_scanned: usize,
    artists_processed: usize,
    cache_hits: usize,
    cache_misses: usize,
    deferred_by_limit: usize,
    metadata_updates: usize,
    releases_checked: usize,
    duplicate_releases: usize,
    review_releases: usize,
    keep_releases: usize,
    unmonitored_releases: usize,
    errors: usize,
}

#[derive(Debug, Serialize)]
struct ArtistReport {
    artist_id: i64,
    artist_name: String,
    status: String,
    fingerprint: String,
    metadata_profile_action: Option<String>,
    duplicate_releases: Vec<Value>,
    review_releases: Vec<Value>,
    keep_releases: Vec<Value>,
    changed_releases: Vec<Value>,
    errors: Vec<String>,
}

#[derive(Debug, Serialize)]
struct RunReport {
    workflow: &'static str,
    run_id: String,
    mode: String,
    started_at: String,
    finished_at: String,
    report_path: String,
    summary: Summary,
    artists: Vec<ArtistReport>,
}

pub async fn run(args: AlbumArgs) -> Result<()> {
    let started_at = now_iso();
    let run_id = run_id("album-cleanup");
    let app = AppConfig::from_env()?;
    let cfg = AlbumConfig::from_env(&args);
    let lidarr = Lidarr::new(
        app.lidarr_base_url.clone(),
        app.lidarr_header_value.clone(),
        cfg.concurrency,
    );
    let store = Store::open(&app.cache_db).await?;

    ensure_profile_exists(&lidarr, cfg.target_metadata_profile_id).await?;

    let cache = store.artist_cache().await?;
    let mut artists = lidarr.artists().await?;
    artists.sort_by_key(|a| id(a).unwrap_or_default());

    let eligible = artists
        .into_iter()
        .filter(|a| args.artist_id.map(|x| id(a) == Some(x)).unwrap_or(true))
        .filter(|a| !cfg.only_monitored_artists || bool_field(a, "monitored") == Some(true))
        .collect::<Vec<_>>();

    let mut selected = Vec::new();
    let mut summary = Summary {
        artists_scanned: eligible.len(),
        ..Summary::default()
    };
    for artist in &eligible {
        let artist_id = id(artist).unwrap_or_default();
        if artist_id == 0 {
            continue;
        }
        let fp = artist_fingerprint(artist, &cfg);
        if !args.force
            && cache
                .get(&artist_id)
                .map(|row| cache_hit(row, &fp, cfg.cache_days))
                .unwrap_or(false)
        {
            summary.cache_hits += 1;
            continue;
        }
        if args.artist_id.is_none() && selected.len() >= cfg.max_artists_per_run {
            summary.deferred_by_limit += 1;
            continue;
        }
        summary.cache_misses += 1;
        selected.push((artist.clone(), fp));
    }

    let reports = stream::iter(selected.into_iter().map(|(artist, fp)| {
        let lidarr = lidarr.clone();
        let store = store.clone();
        let cfg = cfg.clone();
        async move { process_artist(&lidarr, &store, &cfg, artist, fp).await }
    }))
    .buffer_unordered(cfg.concurrency)
    .collect::<Vec<_>>()
    .await;

    let mut artists = Vec::new();
    for result in reports {
        match result {
            Ok(report) => {
                summary.artists_processed += 1;
                summary.metadata_updates += usize::from(report.metadata_profile_action.is_some());
                summary.releases_checked += report.duplicate_releases.len()
                    + report.review_releases.len()
                    + report.keep_releases.len();
                summary.duplicate_releases += report.duplicate_releases.len();
                summary.review_releases += report.review_releases.len();
                summary.keep_releases += report.keep_releases.len();
                summary.unmonitored_releases += report.changed_releases.len();
                summary.errors += report.errors.len();
                artists.push(report);
            }
            Err(_err) => summary.errors += 1,
        }
    }

    let mut report = RunReport {
        workflow: "Album duplicates cleanup",
        run_id: run_id.clone(),
        mode: if cfg.apply { "apply" } else { "dry-run" }.into(),
        started_at,
        finished_at: now_iso(),
        report_path: String::new(),
        summary,
        artists,
    };
    let path = write_json_report(&app.report_dir, "album-cleanup", &run_id, &report).await?;
    report.report_path = path.display().to_string();
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn ensure_profile_exists(lidarr: &Lidarr, profile_id: i64) -> Result<()> {
    let profiles = lidarr.metadata_profiles().await?;
    if profiles.iter().any(|p| id(p) == Some(profile_id)) {
        Ok(())
    } else {
        Err(anyhow!("metadata profile {profile_id} not found"))
    }
}

async fn process_artist(
    lidarr: &Lidarr,
    store: &Store,
    cfg: &AlbumConfig,
    artist: Value,
    fingerprint: String,
) -> Result<ArtistReport> {
    let artist_id = id(&artist).ok_or_else(|| anyhow!("artist without id"))?;
    let name = artist_name(&artist);
    let current_profile = artist
        .get("metadataProfileId")
        .and_then(Value::as_i64)
        .or_else(|| {
            artist
                .pointer("/metadataProfile/id")
                .and_then(Value::as_i64)
        });

    let mut report = ArtistReport {
        artist_id,
        artist_name: name.clone(),
        status: "success".into(),
        fingerprint: fingerprint.clone(),
        metadata_profile_action: None,
        duplicate_releases: vec![],
        review_releases: vec![],
        keep_releases: vec![],
        changed_releases: vec![],
        errors: vec![],
    };

    if current_profile != Some(cfg.target_metadata_profile_id) {
        report.metadata_profile_action =
            Some(if cfg.apply { "updated" } else { "would_update" }.into());
        if cfg.apply {
            let mut updated = artist.clone();
            updated["metadataProfileId"] = json!(cfg.target_metadata_profile_id);
            lidarr
                .put(&format!("/artist/{artist_id}"), &updated)
                .await?;
            let _ = lidarr.refresh_artist(artist_id).await;
        }
        store
            .upsert_artist(&ArtistCache {
                artist_id,
                fingerprint,
                processed_at: now_iso(),
                status: "success".into(),
            })
            .await?;
        return Ok(report);
    }

    let albums = lidarr.albums_for_artist(artist_id).await?;
    let tracks = lidarr.tracks_for_artist(artist_id).await?;
    let mut tracks_by_album: std::collections::HashMap<i64, Vec<Value>> =
        std::collections::HashMap::new();
    for track in tracks {
        if let Some(album_id) = track_album_id(&track) {
            tracks_by_album.entry(album_id).or_default().push(track);
        }
    }

    let full_albums = albums
        .iter()
        .filter(|a| album_type(a) == "album")
        .cloned()
        .collect::<Vec<_>>();
    let mut candidate_releases = albums
        .iter()
        .filter(|a| matches!(album_type(a).as_str(), "single" | "ep"))
        .cloned()
        .collect::<Vec<_>>();
    if cfg.only_monitored_releases {
        candidate_releases.retain(|a| bool_field(a, "monitored") == Some(true));
    }

    let full_track_index = build_full_track_index(&full_albums, &tracks_by_album);
    for release in candidate_releases {
        let release_id = id(&release).unwrap_or_default();
        let release_tracks = tracks_by_album
            .get(&release_id)
            .cloned()
            .unwrap_or_default();
        let decision = classify_release(
            &release,
            &release_tracks,
            &full_track_index,
            cfg.duration_tolerance_ms,
        );
        let item = json!({
            "releaseId": release_id,
            "releaseTitle": album_title(&release),
            "releaseType": album_type(&release),
            "decision": decision,
            "trackCount": release_tracks.len(),
        });
        match decision {
            "DUPLICATE" => report.duplicate_releases.push(item.clone()),
            "REVIEW" => report.review_releases.push(item),
            _ => report.keep_releases.push(item),
        }
        if decision == "DUPLICATE" && cfg.apply && cfg.auto_unmonitor_duplicates && release_id > 0 {
            let mut album = lidarr.get(&format!("/album/{release_id}")).await?;
            album["monitored"] = json!(false);
            if let Err(err) = lidarr.put(&format!("/album/{release_id}"), &album).await {
                report.errors.push(err.to_string());
            } else {
                report
                    .changed_releases
                    .push(json!({ "releaseId": release_id, "action": "unmonitored" }));
            }
        }
    }

    if !report.errors.is_empty() {
        report.status = "error".into();
    }
    store
        .upsert_artist(&ArtistCache {
            artist_id,
            fingerprint,
            processed_at: now_iso(),
            status: report.status.clone(),
        })
        .await?;
    Ok(report)
}

fn build_full_track_index(
    full_albums: &[Value],
    tracks_by_album: &std::collections::HashMap<i64, Vec<Value>>,
) -> std::collections::HashMap<String, Vec<i64>> {
    let mut index: std::collections::HashMap<String, Vec<i64>> = std::collections::HashMap::new();
    for album in full_albums {
        if let Some(album_id) = id(album) {
            for track in tracks_by_album.get(&album_id).into_iter().flatten() {
                index
                    .entry(normalize(&track_title(track)))
                    .or_default()
                    .push(duration_ms(track).unwrap_or_default());
            }
        }
    }
    index
}

fn classify_release(
    release: &Value,
    tracks: &[Value],
    full_track_index: &std::collections::HashMap<String, Vec<i64>>,
    tolerance_ms: i64,
) -> &'static str {
    if tracks.is_empty() {
        return "REVIEW";
    }
    let mut matched = 0;
    let mut needs_review = 0;
    for track in tracks {
        let key = normalize(&track_title(track));
        let duration = duration_ms(track).unwrap_or_default();
        match full_track_index.get(&key) {
            Some(durations)
                if durations
                    .iter()
                    .any(|d| (*d - duration).abs() <= tolerance_ms) =>
            {
                matched += 1
            }
            Some(_) => needs_review += 1,
            None => return "KEEP",
        }
    }
    if matched == tracks.len() {
        "DUPLICATE"
    } else if needs_review > 0 {
        "REVIEW"
    } else {
        let _ = release;
        "KEEP"
    }
}

fn normalize(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn artist_fingerprint(artist: &Value, cfg: &AlbumConfig) -> String {
    let value = json!({
        "artistId": id(artist),
        "artistName": artist_name(artist),
        "monitored": bool_field(artist, "monitored"),
        "metadataProfileId": artist.get("metadataProfileId").or_else(|| artist.pointer("/metadataProfile/id")),
        "settings": {
            "targetMetadataProfileId": cfg.target_metadata_profile_id,
            "durationToleranceMs": cfg.duration_tolerance_ms,
            "autoUnmonitorDuplicates": cfg.auto_unmonitor_duplicates,
        }
    });
    let mut h = Sha256::new();
    h.update(value.to_string().as_bytes());
    format!("{:x}", h.finalize())
}

fn cache_hit(row: &ArtistCache, fingerprint: &str, max_age_days: i64) -> bool {
    if row.status != "success" || row.fingerprint != fingerprint {
        return false;
    }
    DateTime::parse_from_rfc3339(&row.processed_at)
        .map(|dt| {
            Utc::now()
                .signed_duration_since(dt.with_timezone(&Utc))
                .num_days()
                <= max_age_days
        })
        .unwrap_or(false)
}
