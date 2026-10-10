# Owner D: mutators and damage

Pinned oracle: `~/src/pokemon-showdown`, commit `7332b60e22b9e8194bb53549549eba241d73cc9a`. Showdown was read only. Work stayed in the `gpt/mutators` worktree; no dependencies or network access were added.

## Completed

All 58 original TODOs in `actions/mutators/` and all seven original TODOs in `actions/damage/` are implemented. The earlier stat/read-query implementations are retained.

- HP: battle damage, spread damage, direct damage, heal; raw damage/heal/sethp; faint queueing and frozen attribution.
- Boosts: sparse insertion order, ChangeBoost, capping, TryBoost, per-property application/logging, AfterEachBoost, AfterBoost, turn flags; set/clear/raw boost operations.
- Major status and volatile installation, restart, immunity, duration, rollback, retirement and linked cleanup.
- Side/slot conditions, weather, terrain and pseudo-weather installation/removal and effective queries.
- Item take/set/use/eat/consume/clear, lastItem and consumption flags; ability replacement and Skill Swap End/Start ordering.
- Species/forme changes, max-HP updates, Transform, volatile transfer/reset, types, disabled move slots, PP deductions and attack records.
- Effectiveness, type/status immunity, fixed/ordinary/spread/confusion damage, crits, STAB/Tera/Stellar, stat selection, weather/spread/burn/final modifiers and 16-bit damage truncation.

Mutation helpers and numeric-move scratch storage allocate no heap objects. Logging stays structured and is skipped by NoLog's existing generic path. No throughput benchmark is claimed while the complete move/lifecycle pipeline is still under implementation.

## Semantics worth preserving

- JS `trunc` is unsigned conversion, including negative/NaN/infinite inputs. A negative *raw* heal can wrap and heal to max HP; battle heal first turns a truthy amount <= 1 into 1. NaN raw `sethp` becomes 1 HP because truncation precedes the NaN check.
- Raw `boost_by` returns the last actual delta, not their sum. Zero requested boost keys disappear during capping; capped nonzero requests can retain a zero delta. Flags inspect the sparse object after AfterBoost's in-place mutations.
- A null Damage relay vetoes without becoming numeric zero. Spread calculations turn null/false failures into false; falsy targets preserve incoming results. Substitute null targets initially retain true, then spreadDamage turns that entry into numeric zero.
- The pinned getSpreadDamage sets its entry to undefined before checking the old false value; its nested `-fail` branch is consequently unreachable.
- Pokemon/health attribution defaults from event.source; item/field methods default from event.target. Side and pseudo-weather installation do not silently supply the current source effect. Duration callbacks use `call_hook` directly and inherit the caller's effect/state context, rather than a new single-event frame.
- Showdown retains initial `{id: ''}` event/effect objects. Mutation-boundary `Dex(NONE)` preserves a truthy empty effect and enables onEffect Damage without triggering the dispatcher's missing-effect assertion. Initial-effect heal/ability logs use `[from] undefined`; initial boosts can emit ordinary boost lines. This view has no hooks; its shared manifest type is Condition, whereas TS's initial object has no effectType property. New callbacks examining that exact distinction would need a dedicated shared initial-effect view.
- Cell effectOrder follows the post-increment rules: active nonempty Pokemon states and Side-targeted states increment; targetless field states do not. Failed starts consume their creation order. Cells are replaced/retired without invalidating event pins. Clear item/weather/terrain operations preserve the state object's identity/target where TS does.
- WeatherChange/TerrainChange eachEvent sorts active Pokemon, consuming tie draws even without relevant callbacks.
- Transform installs independent virtual slots with min(5, dex PP); base-slot PP remains untouched and becomes visible again after clearVolatile. Stored target stats are copied after species recalculation, while the speed cache retains the setSpecies value until its normal update.
- Forme max-HP changes preserve missing HP and log silent healing. Permanent detailschange appends the receiver's Tera type, including under Illusion.
- Faint attribution projects live move handles to dex IDs; synthetic and condition views retain their token tags. FAINT_QUEUED prevents duplicate entries. Faint processing remains owner L's responsibility.

## Interfaces and integration conventions

Shared files were not changed. One owned public signature was corrected to preserve TS return distinctions; the new spread helper is additive.

1. Added `get_spread_damage_from(results: TargetResults, targets: Targets, source: MonId, move_handle: MoveHandle) -> TargetResults` in `damage/calculation.rs`. Owner M should use this when it already has TryPrimaryHit results. The original `get_spread_damage` signature remains a convenience wrapper, initializing false targets to false and other entries to true.
2. MoveSlot flags exported from `mutators::pokemon`: `SLOT_DISABLED = 1`, `SLOT_HIDDEN = 2`, `SLOT_USED = 4`, `SLOT_VIRTUAL = 8`. Choices/lifecycle must use those meanings and clear disable/source fields at their specified reset points. True disables are never overwritten by later hidden disables.
3. Pokemon flags exported from `mutators::common`: `USED_ITEM_THIS_TURN = 1 << 22`, `ATE_BERRY = 1 << 23`. Lifecycle must reset the former on switching in/end-turn, as appropriate. ateBerry is persistent. Neither bit currently has a scoped reader.
4. EffectCell.source_effect uses the existing u16 field as an `EffectToken` when installed by addVolatile. Read it with `EffectToken(cell.source_effect.0).resolve()`, so a copied/frozen condition view is retained. Current earlier effects do not read this field directly.
5. Transform's Roost typeWas convention: payload word 0 packs first TypeId in the low byte, second in the next byte. The pending Roost onType implementation must store that pre-filter array. Similarly, copying Dragon Cheer/G-Max Chi Strike payload preserves their effect-owned flags/data if those out-of-scope conditions are later added.
6. Tera Shell's resisted flag is ability-state present bit 8; no payload word is required.
7. **Signature correction:** `skill_swap(source: MonId, target: MonId) -> Relay` replaces the original bool return. It preserves null/false vetoes and returns Undefined on successful completion, exactly as TS. There were no existing callers; owner M must preserve those sentinels. Existing `get_volatile`/`get_slot_condition` APIs still return state CellIds rather than TS Condition objects.
8. An empty old ability returns `Relay::NotFail` (the empty-string relay), not numeric zero. `consume_item` captures the live held item after Eat/Use callbacks; its item argument does not overwrite that bookkeeping.

## Verification

`export PATH=$HOME/.cargo/bin:$PATH; cargo build -j 3; cargo test -j 3` passed: 66 unit tests, six stage1 integration tests, two stage2a integration tests. `git diff --check` passed; no TODO remains in either owned directory.

Oracle scripts and checked-in vectors are under `tools/probes/mutators/`:

| Probe | Rows | Checks |
|---|---:|---|
| raw.cjs | 24 | Raw HP returns, clipping, unsigned conversion, faint queue size |
| state.cjs | 36 | Status/item/ability/boost/condition state, ordering, seeds, command sequence |
| damage.cjs | 968 | Formula, hit metadata, crit/STAB/Tera, modifiers, confusion, final seeds |
| pokemon.cjs | 35 | Formes, stats/max HP/types, Transform/PP restoration, disable flags, volatile copying, condition placement |
| health.cjs | 216 | Battle HP returns, HP, one-shot faint attribution, ordered bracket tags and split entry count |

Run `git -C ~/src/pokemon-showdown rev-parse HEAD` first to verify the pin, then `node tools/probes/mutators/<probe>.cjs` (scripts except raw also accept the oracle checkout path as argument). Fixtures suspend constructor auto-start and omit initial rule pseudo-weather on both sides to isolate the owned APIs. All exercised events/PRNG/callbacks otherwise run through the real methods. Structured test sinks inspect command/tag contracts without substituting production log formatting.

The formula test checks at least 700 vectors; the remaining rows are retained where a move/ability is outside the generated scope or the required callback is pending. State tests exercise 19 currently callable rows, retaining the other vectors for later integration. Pokemon vectors outside the species scope are likewise retained. Field/side/slot mechanics are additionally exercised with existing conditions whose respective field/side hooks are absent, including slot-confusion Start, to verify installation and eachEvent behavior without suppressing required production hooks.

## Remaining gaps outside D

- Registry gaps remain observable: initial Sleep Clause SetStatus, Protect/Substitute/Taunt Start, actual weather/terrain/side condition start/duration callbacks, Facade onBasePower and many other move/effect families. Production code dispatches these normally and the registry still panics when an unported function is reached. No production fallback or registry waiver was added.
- Dex/Recharge getDamage inputs call owner M's get_active_move/release_active_move methods. Their TODOs remain intact in this worktree; tests supply explicit ActiveMove fixtures or exercise D's own numeric synthetic input.
- T's TextLog and formatting/hint/move-edit TODOs remain intact. Complete raw text logs still require T; state/health tests verify structured calls, and failed-status/Skill Swap paths invoke T's methods as required.
- L's faint_messages is called for instafaint; its TODO remains intact. Turn/switch flag reset integration is described above.
- Legacy generation branches, OHKO/Z/Max/Primal/Burst/Staleness machinery and dropped fields (hurtThisTurn/lastDamage/started) remain outside the generated gen9randomdoublesbattle model. The port does not expand that model. The generic permanent Item-forme path does not claim fidelity for those excluded transformations; Roost's pending payload convention must be reconciled when its effect lands.
- Full battle parity and sustained self-play throughput require merging the other owners and completing the effect registry. These tests do not claim end-to-end battle parity yet.

Implementation commits: `0f4263e` (core mutators/immunity), `e55686d` (damage and expanded oracle verification).
