#!/bin/bash
set -euo pipefail

REPO=$(cd "$(dirname "$0")/../.." && pwd)
MAC_REPO=${DISTILL_MAC_REPO:-/Users/aminaliu/.t3/projects/numbrion-mac}
BASE=${DISTILL_BASE:-$MAC_REPO/scratch/distill-loop}
PY=${PY:-/Users/aminaliu/.t3/worktrees/numbrion-mac/t3code-c1f8987c/.venv/bin/python}
LABELS=${DISTILL_LABELS:-$BASE/labels}
TEACHER=${DISTILL_TEACHER:-$BASE/teacher}
STATE=${DISTILL_SYNC_STATE:-$BASE/sync}
CONCURRENCY=${DISTILL_CONCURRENCY:-5}
RELOAD=${DISTILL_RELOAD_MINUTES:-20}
SET_POOL=${DISTILL_SET_POOL:-/Users/aminaliu/.t3/worktrees/numbrion-mac/t3code-c1f8987c/data/teams/pool-s1-200k.txt.gz}
export DISTILL_LABELS="$LABELS" DISTILL_TEACHER="$TEACHER" DISTILL_SYNC_STATE="$STATE"
cd "$REPO"
mkdir -p "$BASE" "$STATE"

alive() {
    [ -f "$1" ] && kill -0 "$(cat "$1")" 2>/dev/null
}

wait_exit() {
    while alive "$1"; do sleep 1; done
    rm -f "$1"
}

case ${1:-status} in
    start)
        if alive "$BASE/labels.pid" || alive "$BASE/sync.pid"; then
            echo "Already running; use status or stop first" >&2
            exit 1
        fi
        "$PY" - "$TEACHER" <<'PY'
import pathlib, sys
if not any(p.is_file() for p in pathlib.Path(sys.argv[1]).glob('*.pt')):
    sys.exit('No finished teacher in ' + sys.argv[1])
PY
        mkdir -p "$LABELS"
        rm -f "$LABELS/STOP"
        nohup "$PY" -u crates/pyengine/search_labels.py --follow "$TEACHER" --minutes inf \
            --reload-minutes "$RELOAD" --set-pool "$SET_POOL" --pool data/teams/train-s42-2000.txt \
            --explore 0.25 --shard 256 --concurrency "$CONCURRENCY" --out "$LABELS" \
            >> "$BASE/labels.log" 2>&1 < /dev/null &
        echo $! > "$BASE/labels.pid"
        nohup "$PY" -u tools/distill/mac_sync.py >> "$BASE/sync.log" 2>&1 < /dev/null &
        echo $! > "$BASE/sync.pid"
        sleep 1
        if ! alive "$BASE/labels.pid" || ! alive "$BASE/sync.pid"; then
            touch "$LABELS/STOP"
            if alive "$BASE/sync.pid"; then kill -TERM "$(cat "$BASE/sync.pid")"; fi
            echo "Startup failed; inspect $BASE/labels.log and $BASE/sync.log" >&2
            exit 1
        fi
        echo "Started label maker $(cat "$BASE/labels.pid") and sync $(cat "$BASE/sync.pid")"
        ;;
    stop)
        mkdir -p "$LABELS"
        touch "$LABELS/STOP"
        wait_exit "$BASE/labels.pid"
        if alive "$BASE/sync.pid"; then
            kill -TERM "$(cat "$BASE/sync.pid")"
            wait_exit "$BASE/sync.pid"
        else
            "$PY" -u tools/distill/mac_sync.py --once >> "$BASE/sync.log" 2>&1 || true
        fi
        echo "Stopped; sync attempted its final round. Check status for any backlog or error."
        ;;
    status)
        for name in labels sync; do
            if alive "$BASE/$name.pid"; then echo "$name pid=$(cat "$BASE/$name.pid") running";
            else echo "$name stopped"; fi
        done
        if [ -f "$BASE/labels.log" ]; then
            rg '^(Status:|Teacher switch:|Teacher:)' "$BASE/labels.log" | tail -2 || true
        fi
        "$PY" - "$LABELS" "$TEACHER" "$STATE" <<'PY'
import json, pathlib, re, sys
labels, teacher, state = map(pathlib.Path, sys.argv[1:])
paths = [p for p in teacher.glob('*.pt') if p.is_file()]
def key(p):
    m = re.fullmatch(r'm(\d+)\.pt', p.name)
    return (1, int(m[1]), p.stat().st_mtime_ns) if m else (0, p.stat().st_mtime_ns, p.name)
newest = max(paths, key=key, default=None)
sent = len(json.loads((state / 'sent.json').read_text())) if (state / 'sent.json').exists() else 0
print(f"shards_made={sum(p.is_file() for p in labels.glob('labels-*.npz'))} shards_sent={sent} teacher={newest.name if newest else 'none'}")
if (state / 'status.json').exists():
    status = json.loads((state / 'status.json').read_text())
    print(f"sync backlog={status['backlog']} last_error={status['last_error'] or 'none'}")
PY
        ;;
    *) echo "Usage: $0 {start|stop|status}" >&2; exit 2 ;;
esac
