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

Do not schedule both jobs at the same time.
