#!/usr/bin/env bash
# Sets up a local Pokemon Showdown server for playing against the bots (docs/play/PLAY.md).
#
#     tools/play/setup-showdown.sh            # creates scratch/showdown-play, installs, builds, writes the config
#     tools/play/setup-showdown.sh --config   # only rewrites scratch/showdown-play/config/config.js
#     tools/play/setup-showdown.sh --config --ladder   # ... with ladder search on (local ladder tests, LADDER.md)
#
# The checkout is pinned to the Showdown commit the engine is tested against. It is cloned from a local checkout
# when one has the commit (SHOWDOWN_SRC, or scratch/pokemon-showdown in this or another worktree of the repo),
# otherwise from GitHub. Environment: SHOWDOWN_PORT (default 8000), SHOWDOWN_BIND (default 127.0.0.1).
set -euo pipefail

COMMIT=7332b60e22b9e8194bb53549549eba241d73cc9a
UPSTREAM=https://github.com/smogon/pokemon-showdown.git
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEST="$REPO/scratch/showdown-play"
PORT="${SHOWDOWN_PORT:-8000}"
BIND="${SHOWDOWN_BIND:-127.0.0.1}"
NOBATTLESEARCH=true
for arg in "$@"; do
    [[ "$arg" == "--ladder" ]] && NOBATTLESEARCH=false
done

write_config() {
    mkdir -p "$DEST/config"
    # Showdown loads config-example.js as defaults and overlays this file (server/config-loader.ts).
    cat > "$DEST/config/config.js" <<EOF
'use strict';
// Local play server for Numbrion bots. Written by tools/play/setup-showdown.sh; rerun it to regenerate.

exports.port = ${PORT};
// Loopback only: noguestsecurity lets anyone take any unregistered name.
exports.bindaddress = '${BIND}';

// Log in with "/trn NAME,0," and no token (bots and the user, no login server needed). The official client still
// gets a signed assertion from play.pokemonshowdown.com, which this server accepts with the default public key.
exports.noguestsecurity = true;
// No time-based throttles (renames, challenges, chat lines) and no IP-based alt or flood checks:
// all bots share one IP and start battles back to back.
exports.nothrottle = true;
exports.noipchecks = true;

// One process: battles of a few local players do not need worker processes.
exports.subprocesses = 0;

exports.reportjoins = false;
exports.reportbattles = false;
exports.reportbattlejoins = false;
exports.forcetimer = false;
exports.backdoor = false;
exports.repl = false;
// Ladder search (/search) is off unless the config was written with --ladder; local ratings then go to config/ladders.
exports.nobattlesearch = ${NOBATTLESEARCH};
exports.autolockdown = false;
exports.crashguardemail = null;
exports.logchallenges = false;
exports.loguserstats = 0;

// Showdown only lets registered or autoconfirmed users send and receive PMs, and every name here is unregistered.
// Mark named users autoconfirmed so the user and the bots can PM (challenge refusals, "challenge me").
exports.startuphook = function () {
	(Chat.handlers.onRename = Chat.handlers.onRename || []).push((user, oldid, newid) => {
		if (newid && !newid.startsWith('guest')) user.autoconfirmed = newid;
	});
};
EOF
    echo "wrote $DEST/config/config.js (port $PORT, bind $BIND)"
}

if [[ "${1:-}" == "--config" ]]; then
    write_config
    exit 0
fi

command -v node >/dev/null || { echo "node is required (22 or later)" >&2; exit 1; }
command -v npm >/dev/null || { echo "npm is required" >&2; exit 1; }

has_commit() { git -C "$1" cat-file -e "${COMMIT}^{commit}" 2>/dev/null; }

if [[ -d "$DEST/.git" ]] && has_commit "$DEST"; then
    echo "using existing $DEST"
else
    rm -rf "$DEST"
    SRC=""
    COMMON="$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
    MAIN="${COMMON:+$(dirname "$COMMON")}"
    shopt -s nullglob
    for cand in "${SHOWDOWN_SRC:-}" "$REPO/scratch/pokemon-showdown" \
            ${MAIN:+"$MAIN/scratch/pokemon-showdown"} ${MAIN:+"$MAIN"/.claude/worktrees/*/scratch/pokemon-showdown}; do
        if [[ -n "$cand" && -d "$cand/.git" ]] && has_commit "$cand"; then SRC="$cand"; break; fi
    done
    shopt -u nullglob
    if [[ -n "$SRC" ]]; then
        echo "cloning from local checkout $SRC"
        git clone --quiet --no-checkout "$SRC" "$DEST"
    else
        echo "cloning from $UPSTREAM"
        git clone --quiet --no-checkout --filter=blob:none "$UPSTREAM" "$DEST"
    fi
fi
git -C "$DEST" -c advice.detachedHead=false checkout --quiet --force "$COMMIT"
echo "checked out $(git -C "$DEST" log --oneline -1)"

cd "$DEST"
# Optional dependencies (sqlite, pg, mail, ...) are only for features a local play server does not use, but esbuild's
# platform binary is optional too and the build needs it: omit them only where the build still works without.
if [[ "$(uname -s)" == MINGW* || "$(uname -s)" == MSYS* || "$(uname -s)" == CYGWIN* ]]; then
    npm ci --no-audit --no-fund --prefer-offline
    # cloud-env (optional) would make the server listen on 0.0.0.0 instead of BIND (server/sockets.ts).
    rm -rf node_modules/cloud-env
else
    npm ci --omit=optional --no-audit --no-fund --prefer-offline
fi
write_config
node build
echo
echo "Showdown is ready in $DEST. Start it with tools/play/start-showdown.sh"
