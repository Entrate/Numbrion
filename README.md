<img src="assets/icon.svg" width="64" height="64" alt="">

# Numbrion

Numbrion is a Rust battle engine for Gen 9 Random Doubles, with a differential-test
harness and a Python self-play API. Active development is on `main`.
See [engine validation](docs/STATUS.md) and [the training API](docs/design/TRAINING-API.md).

For AMD GPU training on Windows, follow [the setup and training guide](docs/training/WINDOWS-GPU.md).
For Apple silicon Macs, see [GPU training on Apple silicon](docs/training/MAC.md).

To play against the bots in the Showdown client, see [Play against the bots](docs/play/PLAY.md); the bots search
one turn ahead ([search](docs/play/SEARCH.md)), and a [Chrome extension](docs/play/EXTENSION.md) shows what a bot
would choose in your battles.

## Build

Install current stable Rust using rustup. Windows requires a matching linker:
Visual Studio C++ tools for MSVC, or MinGW GCC for the GNU toolchain.

```sh
cargo build -p engine --all-features --locked --release
cargo build --manifest-path crates/difftest/Cargo.toml --all-features --locked --release
```

Fair-information search support (world builder, set sampler, revealed-information
tracker) is the opt-in `search` feature (`cargo build -p engine --features search`;
Python wheel: `maturin develop --release --features python,search`). It is off by
default because it adds a small cost to every PRNG draw; see
[the search prerequisites](docs/design/SEARCH-PREREQUISITES.md).

The harness executable is `crates/difftest/target/release/difftest` (`.exe` on
Windows). The engine is a library, with artifacts in `target/release`.

## Verify

The engine's code-generation test needs Node 24 and a built Showdown checkout
pinned to `7332b60e22b9e8194bb53549549eba241d73cc9a`:

```sh
git clone https://github.com/smogon/pokemon-showdown.git scratch/pokemon-showdown
git -C scratch/pokemon-showdown checkout 7332b60e22b9e8194bb53549549eba241d73cc9a
cd scratch/pokemon-showdown
npm ci
npm run build
```

From the Numbrion root, Windows users can build both release artifacts, run the
engine, pyengine (with and without search), and harness tests, replay the
committed fixtures, and run the harness fault-injection checks in one command:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\check.ps1
```

The script uses `scratch/pokemon-showdown` by default. Supply `-ShowdownPath` or
set `NUMBRION_SHOWDOWN` to use another pinned, built checkout. It also discovers
Rust under `%USERPROFILE%\.cargo\bin` and the local Node 24 installation under
`%LOCALAPPDATA%\Programs\numbrion-tools` when the shell's PATH is outdated.

The `reveal-audit` diagnostic runs on battle integration tests and fixture
replays because synthetic unit fixtures bypass reveal bookkeeping.

On other platforms, set `NUMBRION_SHOWDOWN` to the checkout's absolute path and
run:

```sh
cargo test -p engine --features trace-hooks,event-probes,event-trace,search --locked
cargo test -p engine --all-features --locked --test '*'
cargo test -p pyengine --release --locked
cargo test -p pyengine --release --locked --features search
cargo test --manifest-path crates/difftest/Cargo.toml --all-features --locked
crates/difftest/target/release/difftest replay data/fixtures/sample-50.jsonl.gz --sim engine
crates/difftest/target/release/difftest replay data/fixtures/directed-smoke.jsonl.gz --sim engine
crates/difftest/target/release/difftest selftest data/fixtures/sample-50.jsonl.gz
```

See [the harness documentation](docs/design/DIFFTEST.md) for replay options and
[the implementation plan](docs/design/IMPLEMENTATION-PLAN.md) for remaining
engine work.
