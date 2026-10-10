# effects(identity_abilities)

Ports all six records / 16 function sites against Showdown
`7332b60e22b9e8194bb53549549eba241d73cc9a`. Every function site is implemented in
`HOOKS`; there are no waivers or remaining missing-API todos. Initial effect
bodies were committed in `5151be3`; the branch now includes the implemented core
from `engine` at `76a95a3` (merge `aaa5be5`). The pinned checkout was read only.

## Files and semantics

The canonical files are `moves/transform.rs` and `abilities/{asoneglastrier,
asonespectrier,illusion,imposter,trace}.rs` under `crates/engine/src/effects/`.
`asoneglastrier.rs` anchors private `identityabilities/tests.rs` and checked-in
oracle TSVs. Tests share the private test recorder with `formestera`; production
callbacks use existing argument/boost helpers and core identity mutators.

| Effect | Source and behavior |
| --- | --- |
| As One (Glastrier) | `data/abilities.ts:251-267`. Start logs As One then Unnerve and sets unnerved; repeated Start while true is silent. End writes false. FoeTryEatItem returns boolean negation, including true when absent. Move-caused source AfterFaint boosts Atk by the supplied count, attributed to Chilling Neigh. |
| As One (Spectrier) | `data/abilities.ts:275-291`. Same state/log ordering and berry gate, with SpA and Grim Neigh attribution. |
| Illusion | `data/abilities.ts:2057-2091`. BeforeSwitchIn clears old illusion and walks the current party permutation backwards, strictly after the user's current position. Select the last non-fainted member, then stop even if Tera plus Ogerpon/Terapagos disallows that candidate. DamagingHit invokes End with the actual captured ability state. End reveals only while not being called back, clears identity before replace/details formatting, logs Illusion End, then the once-only Illusion Level Mod hint. Faint clears silently. |
| Imposter | `data/abilities.ts:2117-2126`. SwitchIn targets the mirrored opposing active slot (1 minus the user's position in doubles), then calls `transform_into` with the ability effect. It does not select an arbitrary foe. |
| Transform | `data/moves.ts:19837-19839`. OnHit delegates `transform_into(user,target,None)` and returns its exact boolean result. Forme/stat/move/ability copying and transformed/illusion/Substitute rejection remain core-owned. |
| Trace | `data/abilities.ts:5121-5148`. Start sets seek, checks raw opposing No Ability plus Ability Shield, then invokes Update with captured state. Update preserves adjacent-foe order, filters noTrace flags, samples once even with a singleton, and calls setAbility with the selected foe as source. A failed search keeps seeking for a later Update. |

Handlers allocate nothing and format no strings. Trace performs the sole direct
PRNG draw: exactly one `sample` when its candidate list is nonempty. Empty lists
draw nothing. Nested Transform, single-event and boost/ability calls own their
core event draws. All logs use structured `LogEntry`, `LogArg` and tags.

## Payload and core contract

| Files | Payload |
| --- | --- |
| Both As One files | One word: word 0 = unnerved boolean, custom presence bit 8. |
| Trace | One word: word 0 = seek boolean, custom presence bit 8. |
| Illusion, Imposter, Transform | Zero words; Pokemon identity/illusion/transformed state is universal core state. |

No cross-module signature changes were needed for this batch. It relies on
`transform_into`, `set_ability`, party/position and active-slot state,
`single_event` with captured ability state, and the text formatter's illusion
view. The forme batch's small permanent-details correction in `53fd2d1` also
preserves the correct real-mon Tera suffix with illusion. All universal identity
state stays in core rather than effect payloads. No generated files, registry or
other batch effects were edited.

## Tests and oracle gate

`cargo build -j 3` and `cargo test -j 3` pass, including registry/manifest and the
independent assertion that all 16 batch function sites are implemented.

```sh
node tools/probes/identity_abilities/callbacks.mjs
node tools/probes/identity_abilities/scenarios.mjs
cargo test -j 3 --lib abilities_asoneglastrier -- --include-ignored --skip as_one_berries
```

The explicit run passes **11/11** tests: five callback/registry/team checks and
six full-battle fixtures. Callback tables compare 48 Illusion party-selection
cases (permutations, Tera, fainted candidates, Ogerpon/Terapagos), eight Trace
filter/search/sample cases with exact PRNG states, and both As One variants'
Start/restart/End, berry sentinels, logs and two-KO boosts. Full battles check
Illusion reveal and hidden level, mirrored-slot Imposter, successful Transform,
Transform against Illusion failure, singleton Trace and both As One startup
logs. They compare the complete protocol log and final PRNG state. Seeds are
`[101 + row_index,202,303,404]`; exact teams/choices are checked into
`identityabilities/scenarios.tsv`.

Battle tests retain the requested `#[ignore = "needs core"]` annotation and were
explicitly run after the merge. The seventh fixture, `as_one_berries`, cannot yet
complete: another batch's `items:sitrusberry onUpdate` (hook 603) panics during
startup. It remains ignored; berry-gate sentinels for both As One variants are
already checked directly against the actual pinned callback.

Requested end-to-end gate:

```sh
node tools/oracle/gen-directed.mjs --profile batch:identity_abilities --count 100 --threads 2 --out /tmp/identity.jsonl.gz --verify
cd crates/difftest
cargo build --release -j 3
./target/release/difftest replay /tmp/identity.jsonl.gz --sim engine --jobs 3 --top 20
```

Run seed **1115**: all 100 oracle battles verified; engine **62 passed / 38
other-batch panics**, zero log/request/PRNG divergences. Every exclusion is
`abilities:naturalcure onCheckShow` (hook 340). Adjusted pass rate:
**62/62 = 100%**. Machine-readable replay is `/tmp/identity-replay.json`.

Combined oracle instrumentation replays all 200 requested corpus battles
exactly. Identity invokes **14/16** function sites in those corpora; the absent
sites are the two As One FoeTryEatItem callbacks, covered by the direct oracle
tests. After the other batches land, rerun the 38 excluded Natural Cure battles
and the Sitrus Berry fixture. No unresolved divergence in this batch remains.

The shared core regression corpus `data/fixtures/slice0-200.jsonl.gz` passes
**200/200**, including the core changes documented in the forme report.
