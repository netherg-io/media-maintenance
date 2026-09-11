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

## Homeserver schedules using docker run

The environment file lives on the Docker host at
`/home/nether/media-maintenance/maintenance.env` (mode 0600). Mount it read-only
and pass `--env-file` to the binary: Dokploy's own container does not need to
read the host file. Mount `/srv/media` at `/media` and the host data directory at
`/data`; run as UID/GID 1000 to match media ownership. Use an immutable image tag.

Disk cleanup schedule (daily, 03:00 Europe/Kyiv):

```sh
set -u
image=ghcr.io/netherg-io/media-maintenance:REPLACE_WITH_COMMIT
cleanup_status=0
docker run --rm --name media-maintenance-disk --user 1000:1000 \
  --network dokploy-network \
  --mount type=bind,src=/home/nether/media-maintenance/maintenance.env,dst=/run/maintenance.env,readonly \
  --mount type=bind,src=/srv/media,dst=/media \
  --mount type=bind,src=/home/nether/media-maintenance/data,dst=/data \
  -e DISK_DRY_RUN=false -e DISK_MAX_FILES=1000 -e DISK_MAX_BYTES=53687091200 \
  "$image" --env-file /run/maintenance.env disk-cleanup || cleanup_status=$?
scan_status=0
sleep 6
waited=0
while :; do
  scan_processes=$(docker top media-navidrome-dux02d-navidrome-1 -eo pid,args) || exit 1
  case "$scan_processes" in
    *"/app/navidrome scan"*)
      if [ "$waited" -ge 3600 ]; then exit 1; fi
      sleep 5
      waited=$((waited + 5))
      ;;
    *) break ;;
  esac
done
docker exec -e ND_SCANNER_PURGEMISSING=always \
  media-navidrome-dux02d-navidrome-1 /app/navidrome scan || scan_status=$?
[ "$cleanup_status" -eq 0 ] && [ "$scan_status" -eq 0 ]
```

The schedule waits up to one hour for an existing Navidrome scan to finish.
The incremental rescan purges missing database records without forcing unchanged
tracks to be re-read. Rescan runs even after a partial cleanup failure. It uses the existing Navidrome
container without restarting the service or granting administrator privileges to
an integration account. Its result is retained in the Dokploy schedule log.
The optional in-process Subsonic integration is an alternative for installations
with administrator API credentials.

Quarantine expiration schedule (daily, 12:00 Europe/Kyiv): use the same mounts and
image, container name `media-maintenance-quarantine`, and command
`--env-file /run/maintenance.env quarantine-cleanup`. It retains files for 30 days
from their individual transfer timestamps. Keep reports/manifests for recovery
and auditing. Both commands refuse concurrent access to the same report volume.
