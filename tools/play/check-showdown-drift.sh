#!/usr/bin/env bash
# How far the live Showdown server may have moved from the commit the engine is tested against (docs/play/LADDER.md).
#
#     tools/play/check-showdown-drift.sh            # fetch upstream into scratch/pokemon-showdown and list changes
#
# The main server runs upstream master. Changes to Gen 9 random-battle sets, the simulator or the Gen 9 data can make
# the world builder lose its worlds (the bot then plays its policy) or the team pool miss sets. Re-run before a ladder
# run; if it lists set or mechanics changes, regenerate the pools from an updated checkout and re-check the engine.
set -euo pipefail

PIN=$(sed -n 's/^COMMIT=//p' "$(dirname "${BASH_SOURCE[0]}")/setup-showdown.sh")
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="${SHOWDOWN_SRC:-$REPO/scratch/pokemon-showdown}"
[[ -d "$SRC/.git" ]] || { echo "no Showdown checkout at $SRC (set SHOWDOWN_SRC)" >&2; exit 1; }

git -C "$SRC" fetch --quiet origin master
echo "pinned $(git -C "$SRC" log -1 --format='%h %cd' --date=short "$PIN"), upstream $(git -C "$SRC" log -1 \
    --format='%h %cd' --date=short origin/master), $(git -C "$SRC" rev-list --count "$PIN..origin/master") commits apart"
echo
for paths in "data/random-battles/gen9" "sim" "data/moves.ts data/abilities.ts data/items.ts data/conditions.ts \
data/pokedex.ts data/learnsets.ts data/typechart.ts data/formats-data.ts config/formats.ts"; do
    # shellcheck disable=SC2086
    changes=$(git -C "$SRC" log --format='  %h %cd %s' --date=short "$PIN..origin/master" -- $paths)
    echo "$paths:" | cut -c1-110
    echo "${changes:-  (no changes)}"
done
