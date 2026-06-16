# Configuration

`media-maintenance` is configured entirely through environment variables and command-line flags.

## Required

| Variable | Example | Description |
| --- | --- | --- |
| `LIDARR_BASE_URL` | `http://lidarr:8686/api/v1` | Lidarr API base URL. |
| `LIDARR_HEADER_VALUE` | `<value>` | Header value sent to Lidarr as `X-Api-Key`. |
| `CACHE_DB` | `/data/media-maintenance-cache.json` | JSON cache path for successful artist runs. |
| `REPORT_DIR` | `/data/reports` | Directory where JSON reports are written. |

## Album cleanup

| Variable | Default | Description |
| --- | --- | --- |
| `ALBUM_TARGET_METADATA_PROFILE_ID` | `3` | Target metadata profile id in Lidarr. |
| `ALBUM_APPLY` | `true` | Allows write actions when the command is not run with `--dry-run`. |
| `ALBUM_CONCURRENCY` | `16` | Number of artists processed concurrently. |
| `ALBUM_MAX_ARTISTS_PER_RUN` | `1000` | Upper bound for one run. |
| `ALBUM_CACHE_DAYS` | `30` | Skip artists with a fresh matching cache fingerprint. |
| `ALBUM_DURATION_TOLERANCE_MS` | `5000` | Track duration tolerance when matching duplicates. |
| `ALBUM_PROCESS_ONLY_MONITORED_ARTISTS` | `true` | Ignore unmonitored artists. |
| `ALBUM_PROCESS_ONLY_MONITORED_RELEASES` | `true` | Ignore unmonitored releases when classifying duplicates. |
| `ALBUM_AUTO_UNMONITOR_DUPLICATES` | `true` | Unmonitor duplicate Single/EP releases in apply mode. |

## Disk cleanup

| Variable | Default | Description |
| --- | --- | --- |
| `DISK_DRY_RUN` | `true` | Keep disk cleanup in report-only mode. |
| `DISK_MUSIC_ROOT` | `/media/music` | Mounted host path inside the container. |
| `DISK_LIDARR_MUSIC_ROOT` | `/music` | Path as Lidarr sees it. |
| `DISK_QUARANTINE_ROOT` | `/media/.cleanup-quarantine` | Destination for moved files. |
| `DISK_STALE_HOURS` | `72` | Only move files older than this threshold. |
| `DISK_MAX_FILES` | `100` | Maximum files moved in one run. |
| `DISK_MAX_BYTES` | `26843545600` | Maximum bytes moved in one run. |
| `DISK_SCAN_CONCURRENCY` | `16` | Filesystem/API scan concurrency. |
| `DISK_MOVE_CONCURRENCY` | `2` | Move concurrency in apply mode. |

## Command-line flags

```bash
media-maintenance album-cleanup --dry-run
media-maintenance album-cleanup --artist-id 123 --force
media-maintenance disk-cleanup --dry-run
```

CLI flags override runtime behaviour for the current run only. Environment variables are still the recommended configuration surface for scheduled jobs.
