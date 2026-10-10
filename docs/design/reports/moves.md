# Move execution owner M

Pinned oracle: `7332b60e22b9e8194bb53549549eba241d73cc9a`.
Recovered implementation branch: `worktree-agent-aa8d861be86a3f31d`, resumed from
`c8aea3a`; no recovery edits or commits discarded.

Implemented within `crates/engine/src/actions/moves/`: mutable ActiveMove initialization
and explicit overlay copy/reuse; targeting/location/ally/foe/Dragon Darts queries;
separate hit and Pressure lists plus redirect dispatch; outside and inside callers;
charge/recharge/Struggle wrappers; all eight Gen 9 hit gates; primary/substitute
sentinels; multihit/Loaded Dice and multiaccuracy ordering; declarative hit/self/secondary
payloads; boosts/heal/status/condition installation; force-switch marking; recoil;
DamagingHit/AfterHit/secondary callbacks; Dancer ordering and recursive external moves.
Effect registration, condition callbacks, damage calculations, lifecycle and requests
remain with their assigned owners. There are no reached TODOs in M's directory.

Recovery fixes: reclaim scope-completed global move frames when active globals clear,
retain a nested inside-call frame through the outside caller's AfterMove and Dancer,
and preserve JavaScript Math.round signed zero. Mutable zeros/deletions continue to
use overlay fields, without falling back to immutable Dex data.

Focused verification uses real pinned Showdown TSV captures, not a Rust reconstruction
of the expected mechanics. `tools/probes/moves/{pure,targeting,hit-stages}.mjs` regenerate
them without changing the oracle checkout. The suite includes 43,208 targeting vectors,
220 primitive/rounding/clamping vectors, 654 hit-stage vectors (accuracy stages, spread
roll order, numeric 100 versus true accuracy, smart-target misses, self and secondary
rolls, Pokemon/null/false target distinctions), the existing ally/foe query vectors,
and ActiveMove identity/deletion/lifetime regressions. All assertions include seeds
where the source can draw.

Scope audit of the oracle's 359 declared moves found no beforeMoveCallback,
beforeTurnCallback, priorityChargeCallback, stealsBoosts, OHKO, ignoreAccuracy,
alwaysHit, forceStatus, or static tracksTarget fields. The two queued direct-callback
wrappers deliberately panic for nonexistent callbacks, matching the source's throwing
path; the boost-stealing stage returns undefined because no scoped move supplies it.
tracksTarget set at runtime by scoped abilities remains implemented. Sleep Talk and
Nature Power are absent from the scoped move set; the inside-caller API nevertheless
preserves the undefined/null default order needed by recursive effects such as Magic Bounce.

Integration gates: this recovered worktree still contains L stubs in getAllActive,
faintMessages, canSwitch and queue execution. End-to-end runMove/Dancer/multihit and
nested Magic Bounce/Instruct fixtures therefore run after L and effect owners merge;
none is represented here as passing. Protect/Substitute/Skill Link/Pressure/Loaded Dice
handler registration is a separate effect gate. Full `sample-50` and `fuzz-2000` strict
replay, exact protocol/request bytes and outcome parity remain mandatory integrator
gates. No full-engine performance claim is made.

Build command: direct stable toolchain cargo/rustc, `cargo test -p engine actions::moves
-j 2 --target-dir /tmp/numbrion-moves-target`. Direct binaries avoid the inherited T3
AppImage argv0/rustup-proxy issue. Focused checks: 8 passed, 0 failed (including all recorded vectors).
