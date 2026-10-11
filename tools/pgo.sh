#!/usr/bin/env bash
# Profile-guided build for macOS and Linux (tools/pgo.ps1 is the Windows/MSVC version): the difftest
# binaries (bench, difftest, search_bench, ...) and the pyengine wheel. See docs/design/PGO.md.
#
#   tools/pgo.sh --corpus <training battles .jsonl[.gz]> [--out scratch/pgo] [--train-seconds 20]
#                [--jobs 4] [--pool <team pool .txt[.gz]>] [--maturin <path>] [--python <path>]
#                [--no-wheel] [--native] [--search]
#
# Needs rustup with `rustup component add llvm-tools-preview` (llvm-profdata). The wheel also needs maturin
# (default: on PATH, else <repo>/.venv/bin/maturin) and any Python >= 3.9 to run the instrumented wheel
# (default: the python next to maturin, else python3). Portable builds unless --native; --search builds
# with the opt-in `search` cargo feature. Installs nothing. Rerun after every engine change.
set -euo pipefail

usage() {
    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
}
die() {
    echo "pgo.sh: error: $*" >&2
    exit 1
}
need_value() {
    [ "$#" -ge 2 ] && [ -n "$2" ] || die "$1 needs a value"
}

corpus=''
out='scratch/pgo'
train_seconds=20
jobs=4
pool=''
maturin=''
python=''
wheel=1
native=0
search=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --corpus) need_value "$@"; corpus=$2; shift 2 ;;
        --out) need_value "$@"; out=$2; shift 2 ;;
        --train-seconds) need_value "$@"; train_seconds=$2; shift 2 ;;
        --jobs) need_value "$@"; jobs=$2; shift 2 ;;
        --pool) need_value "$@"; pool=$2; shift 2 ;;
        --maturin) need_value "$@"; maturin=$2; shift 2 ;;
        --python) need_value "$@"; python=$2; shift 2 ;;
        --no-wheel) wheel=0; shift ;;
        --native) native=1; shift ;;
        --search) search=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; die "unknown argument: $1" ;;
    esac
done
[ -n "$corpus" ] || { usage >&2; die "--corpus is required (training battles; never the benchmark corpus)"; }
case "$train_seconds" in ''|*[!0-9]*) die "--train-seconds must be a whole number, got '$train_seconds'" ;; esac
case "$jobs" in ''|*[!0-9]*) die "--jobs must be a whole number, got '$jobs'" ;; esac

repo_root=$(cd "$(dirname "$0")/.." && pwd)
abspath() {
    # Absolute path of an existing file or directory (relative to the caller's directory).
    if [ -d "$1" ]; then (cd "$1" && pwd); else echo "$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"; fi
}
[ -f "$corpus" ] || die "corpus not found: $corpus"
corpus=$(abspath "$corpus")
if [ -n "$pool" ]; then
    [ -f "$pool" ] || die "team pool not found: $pool"
    pool=$(abspath "$pool")
else
    pool="$repo_root/crates/pyengine/testdata/pool-400.txt.gz"
fi
mkdir -p "$out"
out=$(abspath "$out")
cd "$repo_root"

command -v cargo >/dev/null 2>&1 || die "cargo not found on PATH (install Rust with rustup)"
command -v rustc >/dev/null 2>&1 || die "rustc not found on PATH (install Rust with rustup)"
target=$(rustc -vV | sed -n 's/^host: //p')
[ -n "$target" ] || die "could not read the host target from 'rustc -vV'"
sysroot=$(rustc --print sysroot)
profdata="$sysroot/lib/rustlib/$target/bin/llvm-profdata"
if [ ! -x "$profdata" ]; then
    profdata=''
    for cand in "$sysroot"/lib/rustlib/*/bin/llvm-profdata; do
        if [ -x "$cand" ]; then profdata=$cand; break; fi
    done
fi
[ -n "$profdata" ] || die "llvm-profdata not found under $sysroot/lib/rustlib/*/bin; run: rustup component add llvm-tools-preview"

if [ "$wheel" -eq 1 ]; then
    if [ -z "$maturin" ]; then
        if command -v maturin >/dev/null 2>&1; then
            maturin=$(command -v maturin)
        elif [ -x "$repo_root/.venv/bin/maturin" ]; then
            maturin="$repo_root/.venv/bin/maturin"
        else
            die "maturin not found on PATH or in $repo_root/.venv/bin; pass --maturin <path> or --no-wheel"
        fi
    fi
    [ -x "$maturin" ] || die "maturin is not executable: $maturin"
    if [ -z "$python" ]; then
        if [ -x "$(dirname "$maturin")/python3" ]; then
            python="$(dirname "$maturin")/python3"
        elif command -v python3 >/dev/null 2>&1; then
            python=$(command -v python3)
        else
            die "no Python found next to maturin or on PATH; pass --python <path>"
        fi
    fi
    "$python" -c 'import sys; sys.exit(sys.version_info < (3, 9))' 2>/dev/null \
        || die "$python is not a working Python >= 3.9; pass --python <path>"
fi

# Flags go through CARGO_ENCODED_RUSTFLAGS (0x1f-separated) so paths with spaces stay one argument.
sep=$(printf '\037')
extra=''
if [ "$native" -eq 1 ]; then extra="${sep}-Ctarget-cpu=native"; fi
raw="$out/profraw"
merged="$out/merged.profdata"
gen_dir="$out/target-gen"
use_dir="$out/target-use"
# --target keeps build scripts and proc macros (host artifacts) out of RUSTFLAGS, so they are not instrumented.
difftest_build=(build --manifest-path crates/difftest/Cargo.toml --release --locked -j "$jobs" --bins --target "$target")
# The wheel is profiled through itself: a profile only applies to code built exactly like the profiled
# build, and maturin's cdylib build differs from any plain cargo build (crates/pyengine/src/pgo.rs).
wheel_build=(build --release --locked -m crates/pyengine/Cargo.toml --target "$target")
if [ "$search" -eq 1 ]; then
    difftest_build+=(--features engine/search)
    wheel_build+=(--features python,search)
fi

echo "pgo.sh: target $target, corpus $corpus, out $out${extra:+, native CPU}"
unset CARGO_ENCODED_RUSTFLAGS

# 1. Instrumented builds.
rm -rf "$raw"
export CARGO_ENCODED_RUSTFLAGS="-Cprofile-generate=$raw$extra"
cargo "${difftest_build[@]}" --target-dir "$gen_dir"
gen="$gen_dir/$target/release"
if [ "$wheel" -eq 1 ]; then
    rm -rf "$out/wheels-gen"
    "$maturin" "${wheel_build[@]}" --target-dir "$gen_dir" -o "$out/wheels-gen"
fi

# 2. Training workload: NoLog self-play replays (training configuration), a short TextLog pass, the
#    search primitives (restore, clone, reset, leaf turns), and in the instrumented wheel the pyengine
#    workload (BatchEnv self-play with and without TextLog, GridExecutor, clone/restore/replace_hidden_set).
"$gen/bench" "$corpus" --threads 1 --seconds "$train_seconds"
"$gen/bench" "$corpus" --threads 1 --seconds 3 --textlog
for op in restore clone reset leaf; do
    "$gen/search_bench" "$corpus" --op "$op" --seconds 2
done
if [ "$wheel" -eq 1 ]; then
    "$python" tools/pgo_wheel_workload.py "$out/wheels-gen" "$out/wheel-gen" 10 "$pool"
fi

# 3. Merge.
"$profdata" merge -o "$merged" "$raw"

# 4. Optimized build. Profile mismatch warnings for code the workload never runs are harmless.
export CARGO_ENCODED_RUSTFLAGS="-Cprofile-use=$merged$extra"
cargo "${difftest_build[@]}" --target-dir "$use_dir"
echo "PGO binaries: $use_dir/$target/release"

# 5. Wheel, never installed anywhere.
if [ "$wheel" -eq 1 ]; then
    rm -rf "$out/wheels"
    "$maturin" "${wheel_build[@]}" --target-dir "$use_dir" -o "$out/wheels"
    built=$(ls "$out"/wheels/*.whl 2>/dev/null | head -n 1 || true)
    [ -n "$built" ] || die "maturin finished but no wheel was found in $out/wheels"
    echo "PGO wheel: $built"
fi
unset CARGO_ENCODED_RUSTFLAGS
