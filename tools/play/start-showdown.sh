#!/usr/bin/env bash
# Starts the local Showdown server of tools/play/setup-showdown.sh in the foreground (Ctrl-C stops it).
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEST="$REPO/scratch/showdown-play"
if [[ ! -f "$DEST/config/config.js" || ! -d "$DEST/dist" ]]; then
    echo "$DEST is not set up; run tools/play/setup-showdown.sh first" >&2
    exit 1
fi
PORT="$(node -e "console.log(require('$DEST/config/config.js').port || 8000)")"

cat <<EOF
Showdown server on port $PORT. Open the client at

    http://localhost:$PORT/

It redirects to the official client on localhost.psim.us, which connects to ws://localhost:$PORT (use
"localhost", not 127.0.0.1). Then start the bots: .venv/bin/python crates/pyengine/play/server.py

EOF

cd "$DEST"
exec node pokemon-showdown start --skip-build
