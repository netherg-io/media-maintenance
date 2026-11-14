# Dokploy schedules

Production uses short-lived `docker run` jobs. The protected environment file is
`/home/nether/media-maintenance/maintenance.env` (0600), mounted at
`/run/maintenance.env`. Data and reports live under
`/home/nether/media-maintenance/data`; `/srv/media` is mounted at `/media`.
Maintenance containers run as UID/GID 1000. Pin the maintenance image to its Git
commit, and keep `DISK_DRY_RUN=true` in the environment file; only the cleanup
wrapper explicitly overrides it.

CI publishes `scripts/dokploy-refresh.sh` inside the launcher image
`ghcr.io/netherg-io/media-maintenance:orchestrator-<commit>`. It serializes cleanup
and follow-up stages with `flock`, limits daily moves to 1,000 files / 50 GiB, and
processes only durable pending markers. A dry-run or a cleanup with no changes does
not trigger Integrity, Navidrome, or AudioMuse follow-up work.

Dokploy runs its server scripts inside its own container. The launcher gets the
Docker socket and the host directory with the environment file, data and lock
(`MAINTENANCE_HOME`, default `/home/nether/media-maintenance`), and starts the
unprivileged maintenance jobs:

```sh
ac=/sys/class/power_supply/AC/online
[ ! -r "$ac" ] || [ "$(cat "$ac")" = 1 ] || { echo "battery: skip"; exit 0; }
docker run --rm --name media-maintenance-orchestrator \
  --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock \
  --mount type=bind,src=/home/nether/media-maintenance,dst=/home/nether/media-maintenance \
  -e MAINTENANCE_IMAGE=ghcr.io/netherg-io/media-maintenance:REPLACE_WITH_COMMIT \
  ghcr.io/netherg-io/media-maintenance:orchestrator-REPLACE_WITH_COMMIT cleanup
```

Set this schedule to `40 1 * * *`, timezone `Europe/Kyiv`; on battery it skips the night. Enable
`NAVIDROME_EXTERNAL_RESCAN=true`, and configure the Integrity and AudioMuse API
URLs/tokens. Remove `NAVIDROME_URL` when using the external CLI scan.

Use the same launcher **without the trailing `cleanup` argument** for a
`20 1-7 * * *` retry schedule. It only advances pending work from actual earlier
changes; it exits without calling services when nothing is pending. Give this
launcher the distinct container name `media-maintenance-refresh-retry`; the
shared host lock prevents it overlapping cleanup.

The stages are:

1. Integrity incremental scan reconciles removed library results.
2. Navidrome waits for any active scan (up to one hour), then runs
   `/app/navidrome scan` with `ND_SCANNER_PURGEMISSING=always`. No `--full` is used.
   The main server keeps running. The marker is acknowledged only after exit 0.
3. AudioMuse cleaning prunes unavailable tracks. Its task ID is persisted; the
   retry schedule polls it and never cancels or duplicates an active task. A busy
   unrelated task defers the follow-up. AudioMuse starts only after Navidrome
   succeeds. New tracks are left to AudioMuse's native Analysis cron.

Completed stages write a JSON report and clear their own marker. Pending work
survives no-op cleanup, process restarts, and service errors. The original cleanup
failure still makes the schedule fail even if follow-up work succeeds.

## Regular incremental checks

Cleanup starts with an incremental Integrity scan, so no separate `integrity-scan`
schedule is needed. AudioMuse's native **Analysis** cron is enabled at `0 3 * * *`
in its `Europe/Kyiv` timezone; it analyzes only new tracks and skips active main tasks.

## Quarantine retention

Keep `quarantine-cleanup` daily at `0 12 * * *`, timezone `Europe/Kyiv`, with the
same media/data/environment mounts and an immutable image. Files expire 30 days
after their individual transfer timestamps. Expiration does not change the
active library and never creates refresh markers. Keep manifests and reports
for recovery and auditing.
