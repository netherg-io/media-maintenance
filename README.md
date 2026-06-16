# media-maintenance

Rust CLI replacement for Lidarr maintenance workflows migrated from n8n to Dokploy schedules.

## Commands

```bash
media-maintenance album-cleanup
media-maintenance album-cleanup --dry-run --artist-id 123 --force
media-maintenance disk-cleanup --dry-run
```

## Runtime env

Copy `.env.example` in Dokploy and set:

```env
LIDARR_BASE_URL=http://lidarr:8686/api/v1
LIDARR_HEADER_VALUE=<lidarr-header-value>
CACHE_DB=/data/media-maintenance-cache.json
REPORT_DIR=/data/reports
```

## What it does

- `album-cleanup` checks monitored artists, enforces the target metadata profile, detects duplicate Single/EP releases against full albums, caches successful artist runs in a JSON file, and can unmonitor duplicate releases when apply mode is enabled.
- `disk-cleanup` scans the media filesystem, compares it with Lidarr track files/manual import, writes a manifest, and moves selected stale files into quarantine when `DISK_DRY_RUN=false`.

Disk cleanup defaults to dry-run. Keep it that way for the first run.

## CI

GitHub Actions is configured for Blacksmith runners and Blacksmith Docker build actions.
