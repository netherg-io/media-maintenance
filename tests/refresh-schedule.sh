#!/bin/sh
set -eu
repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/bin" "$fixture/home/data/reports"
export MAINTENANCE_HOME="$fixture/home" MAINTENANCE_IMAGE=fixture MOCK_LOG="$fixture/events"
export PATH="$fixture/bin:$PATH"
cat > "$fixture/bin/sleep" <<'MOCK'
#!/bin/sh
exit 0
MOCK
cat > "$fixture/bin/docker" <<'MOCK'
#!/bin/sh
set -eu
case "$1" in
    top) printf 'PID COMMAND\n1 /app/navidrome\n'; exit 0 ;;
    exec)
        echo navidrome >> "$MOCK_LOG"
        [ "${MOCK_FAIL_NAVIDROME:-false}" != true ]
        exit $? ;;
esac
for arg in "$@"; do
    case "$arg" in
        disk-cleanup)
            echo cleanup >> "$MOCK_LOG"
            if [ "${MOCK_CHANGED:-false}" = true ]; then
                for stage in integrity navidrome audiomuse; do echo '{}' > "$MAINTENANCE_HOME/data/reports/pending-$stage.json"; done
            fi ;;
        integrity|navidrome|audiomuse)
            echo "refresh-$arg" >> "$MOCK_LOG"
            rm "$MAINTENANCE_HOME/data/reports/pending-$arg.json" ;;
    esac
done
MOCK
chmod +x "$fixture/bin/docker" "$fixture/bin/sleep"
sh "$repo/scripts/dokploy-refresh.sh"
[ ! -e "$MOCK_LOG" ]
sh "$repo/scripts/dokploy-refresh.sh" cleanup
[ "$(cat "$MOCK_LOG")" = cleanup ]
: > "$MOCK_LOG"
MOCK_CHANGED=true sh "$repo/scripts/dokploy-refresh.sh" cleanup
expected=$(printf 'cleanup\nrefresh-integrity\nnavidrome\nrefresh-navidrome\nrefresh-audiomuse')
[ "$(cat "$MOCK_LOG")" = "$expected" ]
: > "$MOCK_LOG"
sh "$repo/scripts/dokploy-refresh.sh"
[ ! -s "$MOCK_LOG" ]
for stage in navidrome audiomuse; do echo '{}' > "$MAINTENANCE_HOME/data/reports/pending-$stage.json"; done
if MOCK_FAIL_NAVIDROME=true sh "$repo/scripts/dokploy-refresh.sh"; then exit 1; fi
[ "$(cat "$MOCK_LOG")" = navidrome ]
[ -f "$MAINTENANCE_HOME/data/reports/pending-navidrome.json" ]
[ -f "$MAINTENANCE_HOME/data/reports/pending-audiomuse.json" ]
printf 'Refresh scheduling tests passed\n'
