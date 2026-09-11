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

## Command-line flags

```bash
media-maintenance album-cleanup --dry-run
media-maintenance album-cleanup --artist-id 123 --force
media-maintenance disk-cleanup --dry-run
```

CLI flags override runtime behaviour for the current run only. Environment variables are still the recommended configuration surface for scheduled jobs.

## Audio Integrity and quarantine

Set `AUDIO_INTEGRITY_URL` and `AUDIO_INTEGRITY_TOKEN` (the service's dedicated
`API_TOKEN`) to enable corruption/authenticity cleanup. An incremental scan runs
first by default (`AUDIO_INTEGRITY_SCAN=true`); an existing scan is reused and
must finish successfully. `AUDIO_INTEGRITY_TIMEOUT_SECONDS` defaults to 21600.
`AUDIO_INTEGRITY_MUSIC_ROOT` defaults to `/music` and maps the service's paths to
`DISK_MUSIC_ROOT`. Only current validator results matching file size and exact
nanosecond modification time are used. `corrupt` and healthy `likely_lossy`
results qualify; validation errors never qualify as corruption. The existing
stale-age and run limits still apply. Lossy authenticity is a heuristic.

Untracked files are report-only unless `DISK_CLEAN_UNTRACKED=true`; this avoids
moving a file solely because Lidarr does not know it. When enabled, manual-import
API failures abort the run. Known files on unmonitored albums retain their
existing cleanup behavior. Moves are serialized with a plan saved before
moving, an append-only outcome journal synced after each move, and a final snapshot; the previous move-concurrency setting is no
longer used. Files changed during the run are preserved.

`quarantine-cleanup` removes only unchanged, successfully quarantined files
recorded in `moves.json` whose `quarantined_at` timestamp is at least 30 days old.
`quarantine-cleanup --dry-run` previews expiration. Manifests and unrelated files
are retained. Older manifests without a transfer timestamp are never auto-purged.
Disk and quarantine commands share a filesystem lock under `REPORT_DIR`.

Use the global `--env-file /run/maintenance.env` option to read a mounted secret
file. Container environment variables take precedence over this file.

Optional `NAVIDROME_URL`, `NAVIDROME_USER`, and `NAVIDROME_PASSWORD` enable a full
Subsonic rescan after any successful move. This requires an administrator account;
Configure Navidrome `Scanner.PurgeMissing=full` or `always` if absent records
should be removed rather than marked missing. HTTP and Subsonic errors are recorded and cause a nonzero exit after the report
is saved. Scans are polled for completion for up to one hour. A Dokploy schedule
may instead run Navidrome's CLI inside its existing container.
