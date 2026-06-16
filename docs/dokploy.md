# Dokploy deployment

Create a service from this repository Dockerfile.

Mounts:

```text
/srv/media:/media:rw
media-maintenance-data:/data:rw
```

Environment: copy `.env.example`, set `LIDARR_HEADER_VALUE`, and keep `DISK_DRY_RUN=true` for the first run.

Schedules:

```cron
0 0 * * *    media-maintenance album-cleanup
0 4 * * 0    media-maintenance disk-cleanup
```

Recommended first-run commands:

```bash
media-maintenance album-cleanup --dry-run
media-maintenance disk-cleanup --dry-run
```

After reviewing JSON reports in `REPORT_DIR`, enable apply mode by setting `ALBUM_APPLY=true` and `DISK_DRY_RUN=false`.

Do not schedule both jobs at the same time.
