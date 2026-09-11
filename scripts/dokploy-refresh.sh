#!/bin/sh
set -eu
: "${MAINTENANCE_IMAGE:?set the deployed immutable image tag}"
maintenance_home=${MAINTENANCE_HOME:-/home/nether/media-maintenance}
navidrome_container=${NAVIDROME_CONTAINER:-media-navidrome-dux02d-navidrome-1}
reports="$maintenance_home/data/reports"

cleanup_status=0
if [ "${1:-}" != cleanup ]; then
    [ -f "$reports/pending-integrity.json" ] || [ -f "$reports/pending-navidrome.json" ] || [ -f "$reports/pending-audiomuse.json" ] || exit 0
fi

exec 9>"$maintenance_home/refresh.lock"
flock -n 9 || exit 0

run_job() {
    docker run --rm --user 1000:1000 --network dokploy-network \
        --mount "type=bind,src=$maintenance_home/maintenance.env,dst=/run/maintenance.env,readonly" \
        --mount type=bind,src=/srv/media,dst=/media \
        --mount "type=bind,src=$maintenance_home/data,dst=/data" \
        --name media-maintenance-scheduled \
        -e DISK_DRY_RUN=false -e DISK_MAX_FILES=1000 -e DISK_MAX_BYTES=53687091200 \
        "$MAINTENANCE_IMAGE" --env-file /run/maintenance.env "$@"
}

run_refresh() { run_job refresh-after-cleanup "$@"; }

if [ "${1:-}" = cleanup ]; then
    run_job disk-cleanup || cleanup_status=$?
fi

if [ -f "$reports/pending-integrity.json" ]; then
    run_refresh integrity
fi
if [ -f "$reports/pending-navidrome.json" ]; then
    sleep 6
    waited=0
    while :; do
        scan_processes=$(docker top "$navidrome_container" -eo pid,args) || exit 1
        case "$scan_processes" in
            *"/app/navidrome scan"*)
                [ "$waited" -lt 3600 ] || exit 1
                sleep 5
                waited=$((waited + 5))
                ;;
            *) break ;;
        esac
    done
    docker exec -e ND_SCANNER_PURGEMISSING=always "$navidrome_container" /app/navidrome scan
    run_refresh navidrome --navidrome-scanned
fi
if [ -f "$reports/pending-audiomuse.json" ]; then
    run_refresh audiomuse
fi

exit "$cleanup_status"
