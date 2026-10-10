# Lifecycle owner L

Oracle: `/home/aminaliu/src/pokemon-showdown` at
`7332b60e22b9e8194bb53549549eba241d73cc9a`. Recovered branch base `c20efdb`;
checkpoint `9e6d07d` preserved the stopped session's uncommitted vector tests.
Reviewed `sim/battle-queue.ts`, `sim/battle.ts`, and `sim/battle-actions.ts`
against every method in `crates/engine/src/sim/lifecycle/`. No implementation
stubs remain in L's directory.

Implemented behavior includes queue expansion, exact tie sorting/inclusive insertion,
priority edits, saved midturn queues, start/runSwitch batching, party permutation,
switch-out/drag sequencing, Revival Blessing, generic and species-specific Tera
control flow, endTurn resets and request dispatch, residual HP captures,
forced replacements, faint order and win checks, terminal results and turn limits.
Before-turn/priority-charge execution wrappers delegate to M; scoped moves define
neither callback, so their absence in queue expansion is a verified scope fact.

Validation on engine `5e328b5`, separate target directory, `cargo test -p engine
-j 2 sim::lifecycle -- --nocapture`: **5 passed, zero ignored/failures**.

| Oracle fixture | Cases/rows | Coverage |
|---|---:|---|
| start.tsv | 600 | 300 initial switch/runSwitch sequences, ability/item order and RNG |
| queue.tsv | 3,697 | sort, insert ties, cancel, prioritize, change, speed edits |
| flow.tsv | 8,936 | switch/instaswitch, drag, replacement, generic Tera, Revival, residual/endTurn, double KO |
| move.tsv | 11,710 | queue target/RNG/priority, Struggle/Recharge, Tera expansion, dynamic re-sort, midturn faints |
| boundary.tsv | 290 | exact endpoint logs/state/seeds/outcomes; faint order/last-first, duplicate faint, totalFainted cap, Tera then faint, tiebreak, turn limits |

Total: **25,233** checked rows/cases. Existing start/flow/move oracle probes were
regenerated and were byte-identical. New boundary probe generates its fixture from
the pinned real methods without intercepting effect callbacks.

Flow and move tests deliberately isolate lifecycle/queue behavior: their Node
probes replace makeRequest with a request-kind recorder and move execution with
a no-op that may enqueue a faint. Rust uses the identical interception via a
`cfg(test)`, thread-scoped recorder (restored on scope exit). Production dispatch
still calls C's real make_request. These are **isolated checks**, not claims that
requests or entire battles pass replay.

`startup-vectors.mjs` additionally records **50 full startup boundaries** with no
interception: real rule hooks/endTurn, exact normalized protocol and both request
JSON byte strings. `full_startup_matches_pinned_showdown` is currently an explicit
ignored integration gate while the six rule Begin callbacks are not registered;
enable/run after the lead's rule implementation lands. Species-specific Tera
callbacks require the formes/identity effect batches. Choice commit/end-to-end
move behavior, residual callback Emergency Exit interactions, full sample-50 and
fuzz strict-order replay remain lead integration gates; they are not counted as
passing here. M owns pending-release scratch cleanup at request boundaries.

Changes `9100066` added endpoint acceptance fixtures; the following validation
commit activates isolated flow/move suites and adds the explicit full startup gate.
