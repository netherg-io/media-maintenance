# Dokploy schedules

Production uses short-lived `docker run` jobs. The protected environment file is
`/home/nether/media-maintenance/maintenance.env` (0600), mounted at
`/run/maintenance.env`. Data and reports live under
`/home/nether/media-maintenance/data`; `/srv/media` is mounted at `/media`.
Maintenance containers run as UID/GID 1000. Pin the maintenance image to its Git
commit, and keep `DISK_DRY_RUN=true` in the environment file; only the cleanup
wrapper explicitly overrides it.

Install `scripts/dokploy-refresh.sh` on the Docker host as
`/home/nether/media-maintenance/refresh.sh`. It serializes cleanup and follow-up
stages with `flock`, limits daily moves to 1,000 files / 50 GiB, and processes
only durable pending markers. A dry-run or a cleanup with no changes does not
trigger Integrity, Navidrome, or AudioMuse follow-up work.

Dokploy runs its server scripts inside its own container, so host files are not
directly visible to its shell. Use a Docker CLI launcher with the host directory
and Docker socket mounted; it launches the unprivileged maintenance jobs:

```sh
docker run --rm --name media-maintenance-orchestrator \
  --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock \
  --mount type=bind,src=/home/nether/media-maintenance,dst=/home/nether/media-maintenance \
  -e MAINTENANCE_IMAGE=ghcr.io/netherg-io/media-maintenance:REPLACE_WITH_COMMIT \
  docker@sha256:000bb62ff495f986c9f5578eb67cc2cb98b91138eda81d7762d5371eb8a497fe \
  sh /home/nether/media-maintenance/refresh.sh cleanup
```

Set this schedule to `0 3 * * *`, timezone `Europe/Kyiv`. Enable
`NAVIDROME_EXTERNAL_RESCAN=true`, and configure the Integrity and AudioMuse API
URLs/tokens. Remove `NAVIDROME_URL` when using the external CLI scan.

Use the same launcher **without the trailing `cleanup` argument** for a
`*/15 * * * *` retry schedule. It only advances pending work from actual earlier
changes; it exits without calling services when nothing is pending. Give this
launcher the distinct container name `media-maintenance-refresh-retry`; the
shared host lock prevents it overlapping cleanup.

The stages are:

1. Integrity incremental scan reconciles removed library results.
2. Navidrome waits for any active scan (up to one hour), then runs
   `/app/navidrome scan` with `ND_SCANNER_PURGEMISSING=always`. No `--full` is used.
   The main server keeps running. The marker is acknowledged only after exit 0.
3. AudioMuse cleaning prunes unavailable tracks, then incremental analysis
   skips already analyzed tracks. Its task IDs are persisted; the retry schedule
   polls them and never cancels or duplicates an active task. A busy unrelated
   task defers the follow-up. AudioMuse starts only after Navidrome succeeds.

Completed stages write a JSON report and clear their own marker. Pending work
survives no-op cleanup, process restarts, and service errors. The original cleanup
failure still makes the schedule fail even if follow-up work succeeds.

## Regular incremental checks

Schedule `integrity-scan` daily at `0 2 * * *`, timezone `Europe/Kyiv`, using:

```sh
docker run --rm --name media-maintenance-integrity --user 1000:1000 \
  --network dokploy-network \
  --mount type=bind,src=/home/nether/media-maintenance/maintenance.env,dst=/run/maintenance.env,readonly \
  --mount type=bind,src=/home/nether/media-maintenance/data,dst=/data \
  ghcr.io/netherg-io/media-maintenance:REPLACE_WITH_COMMIT \
  --env-file /run/maintenance.env integrity-scan
```

This reuses an already-running scan and cached file validations. It does not
schedule follow-up Navidrome or AudioMuse scans. AudioMuse's native **Analysis**
cron is enabled at `0 4 * * *` in its existing `Europe/Kyiv` timezone; native cron
uses incremental analysis across the whole library and skips active main tasks.

## Quarantine retention

Keep `quarantine-cleanup` daily at `0 12 * * *`, timezone `Europe/Kyiv`, with the
same media/data/environment mounts and an immutable image. Files expire 30 days
after their individual transfer timestamps. Expiration does not change the
active library and never creates refresh markers. Keep manifests and reports
for recovery and auditing.
