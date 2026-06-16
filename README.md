# media-maintenance

[![CI](https://github.com/netherg-io/media-maintenance/actions/workflows/ci.yml/badge.svg)](https://github.com/netherg-io/media-maintenance/actions/workflows/ci.yml)
![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)
![Container](https://img.shields.io/badge/container-GHCR-blue.svg)

A small Rust CLI for running safe, scheduled Lidarr maintenance jobs from containers.

It replaces several heavy n8n workflows with short-lived Dokploy cron jobs: start cold, inspect Lidarr and the media filesystem, write JSON reports, and exit.

## Why

Long-running automation tools are great for orchestration, but large Lidarr libraries are easier to maintain with a compiled CLI:

- bounded concurrency instead of unbounded workflow fan-out;
- deterministic JSON reports for every run;
- dry-run first, apply later;
- container-native deployment;
- no always-on worker process.

## Features

- **Album cleanup**
  - checks monitored artists;
  - enforces a target Lidarr metadata profile;
  - detects duplicate Single/EP releases against full album tracks;
  - optionally unmonitors duplicate releases;
  - caches successful artist runs in a JSON cache file.

- **Disk cleanup**
  - scans the mounted music directory;
  - compares filesystem files with Lidarr state and manual import data;
  - writes a manifest/report;
  - moves selected stale files into quarantine only when dry-run is disabled.

- **Ops-friendly runtime**
  - one static command-line entrypoint;
  - Docker/GHCR publishing in CI;
  - designed for Dokploy scheduled jobs;
  - reports and cache stored under `/data`.

## Quick start

Pull the image:

```bash
docker pull ghcr.io/netherg-io/media-maintenance:latest
```

Run album cleanup in dry-run mode:

```bash
docker run --rm \
  --env-file ./media-maintenance.env \
  -v /srv/media:/media:rw \
  -v media-maintenance-data:/data:rw \
  ghcr.io/netherg-io/media-maintenance:latest \
  album-cleanup --dry-run
```

Run disk cleanup in dry-run mode:

```bash
docker run --rm \
  --env-file ./media-maintenance.env \
  -v /srv/media:/media:rw \
  -v media-maintenance-data:/data:rw \
  ghcr.io/netherg-io/media-maintenance:latest \
  disk-cleanup --dry-run
```

## Commands

```bash
media-maintenance album-cleanup
media-maintenance album-cleanup --dry-run
media-maintenance album-cleanup --dry-run --artist-id 123 --force
media-maintenance disk-cleanup --dry-run
media-maintenance disk-cleanup
```

## Configuration

Copy `.env.example` and set at least:

```env
LIDARR_BASE_URL=http://lidarr:8686/api/v1
LIDARR_HEADER_VALUE=<lidarr-header-value>
CACHE_DB=/data/media-maintenance-cache.json
REPORT_DIR=/data/reports
```

See [configuration docs](docs/configuration.md) for all environment variables.

## Deployment

The intended deployment target is a scheduled container job:

```cron
0 0 * * *    media-maintenance album-cleanup
0 4 * * 0    media-maintenance disk-cleanup
```

See [Dokploy deployment](docs/dokploy.md) for the full runbook.

## Safety model

Disk cleanup defaults to dry-run. Keep it that way until you have reviewed the generated reports.

Recommended rollout:

1. run `album-cleanup --dry-run`;
2. run `disk-cleanup --dry-run`;
3. review JSON reports in `REPORT_DIR`;
4. enable album apply mode;
5. enable disk cleanup only after validating the quarantine manifest.

## Development

```bash
cargo fmt --check
cargo check --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
```

## License

MIT
