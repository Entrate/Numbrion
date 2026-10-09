# Core Pokemon stat, speed, weight and hit-data queries

Branch `finish/core-queries`, starting from completed query/suppression baseline
`77d57e7` (including root prerequisites `63e7304`, `4f37a05`). Authoritative
read-only oracle: Showdown `7332b60e22b9e8194bb53549549eba241d73cc9a`.
This slice changes only `actions/damage/stats.rs`, its private `stats/` fixtures and
this report. All seven previously declared APIs are implemented without signature
changes.

## Source behavior

- `calculate_stat` (`pokemon.ts:560-594`) swaps base defenses under Wonder Room,
  invokes ModifyBoost on a fresh sparse one-key object targeted at `stat_user` or
  the original Pokemon, clamps stages to -6..6, floors boosted values, and applies
  the existing verified fixed-point modifier. Zero/NaN/absent modifiers select 1,
  as in `modifier || 1`. A deleted boost key preserves the source's NaN arithmetic
  rather than silently inventing a zero stage.
- `get_stat` (`pokemon.ts:596-639`) reads the original base first. Its unmodified
  Wonder Room path swaps only the boost key; the ordinary path retains the
  defense event name for the room's ModifyDef/SpD callbacks. The ordinary query
  copies all seven boost keys before ModifyBoost and invokes the matching stat
  event. `unboosted`/`unmodified` retain their distinct event/boost gates. Speed
  above 10,000 is capped in this fixed format.
- `pokemon_action_speed` (`pokemon.ts:641-649`) applies ordinary Trick Room,
  then the source uint32/13-bit truncation. `update_pokemon_speed`
  (`pokemon.ts:556-558`) caches that action speed, including reversal/wrapping.
- `get_best_stat` (`pokemon.ts:656-668`) starts with Atk and zero, considers stats
  in Atk/Def/SpA/SpD/Spe order, and repeats `get_stat` whenever a new maximum is
  found. It deliberately does not reuse the comparison result.
- `get_weight` (`pokemon.ts:685-688`) invokes ModifyWeight and returns at least
  1 hectogram. NaN remains NaN, matching Math.max rather than Rust's NaN-skipping
  floating-point max behavior.
- `move_hit_data` (`pokemon.ts:520-523,706-714`) initializes crit/typeMod/protect
  metadata lazily inside the existing mutable ActiveMove. Its key is current
  slot (`side*6+position`), rather than stable Pokemon identity. Replacements in
  the same slot reuse the metadata; a different move frame has independent data.

ModifyBoost's original scratch object and any replacement are both reclaimed
once their values have been copied. No query allocates, normalizes strings or
introduces direct PRNG draws. Missing/non-object ModifyBoost returns and
non-numeric stat/weight relays fail explicitly; the executable pinned callbacks
have the required numeric/object contracts.

## Verification

`stats/oracle.cjs` verifies the checkout SHA and executes actual pinned
Battle/Pokemon methods. Only constructor automatic start is suspended; fixtures
set active slots and relevant state explicitly. No stat query or event dispatcher
is replaced. `stats/vectors.tsv` records **2,277 comparisons**:

- 840 calculateStat cases across all five stats, stage boundaries -128..127,
  sparse zero boosts, absent/zero/NaN/fractional/negative/infinite modifiers and
  default/explicit-other stat-user targets.
- 1,280 getStat cases across all five stats, four boost stages, all four option
  combinations, active/inactive suppression, and real Huge Power, Choice Band,
  Assault Vest and Choice Scarf callbacks.
- 108 speed cases across 0/1, 13-bit wrap boundaries, 10,000 clamp boundaries and
  the u16 state maximum, with boosts, Choice Scarf and Trick Room. Both returned
  action speed and refreshed cached speed match the source.
- 32 best-stat cases cover ties, zero stats and increasing/decreasing maxima with
  option gates and real stat modifiers.
- 16 weight cases cover zero/minimum/large weights, Heavy Metal and suppression.
- One hit-data source fixture checks same-slot identity, current-position changes,
  restoration, independent opposing slots and preserved metadata.

Additional Rust tests cover same-slot replacement sharing, distinct move frames,
missing boost-key NaN arithmetic, extreme stage clamps and sparse present-zero
object shape through the actual no-handler ModifyBoost event. Every vector checks
its exact source PRNG seed, unchanged protocol count and no leaked boost relay,
event frame or handler buffer. Pokemon boosts remain unchanged by getStat.

Huge Power priority 5 and Choice Band priority 1 do **not** tie. Their current
best-stat vectors verify output/order/seeds, but do not by themselves prove the
repeated-call count. A meaningful follow-up integration gate is two tied Unaware
ModifyBoost listeners: their ordering draws will distinguish the repeated source
queries. Only Unaware supplies the generated ModifyBoost callback, and its effect
file is not present in this worktree baseline. The implementation retains the
exact repeated calls now; no test-only production substitute fills that dependency.

Validation uses `/bin/bash`, `login=false`, Cargo offline and one Rust job:

```text
cargo check -p engine -j 1 --offline
cargo test -p engine -j 1 --offline actions::damage::stats -- --nocapture
cargo test -p engine -j 1 --offline
```

Focused tests: **3 pass**, including all 2,277 vectors. Full engine validation:
**48 unit + 6 Stage 1 + 2 Stage 2A tests pass**, zero failures or ignored tests.
`git diff --check` passes. No full-battle replay or performance claim is made.

## Scope and integration

Root's real event dispatcher, suppression queries, fixed-point modifier and live
ActiveMove accessors are required and used. Any reached pending callback retains
its explicit production failure. Root/other agents own the remaining effects,
move materialization and complete battle execution.

Wonder Room is absent from this generated executable Dex. Its source key branch
and the calculateStat/getStat base-versus-boost distinction remain explicit,
without fabricated IDs. No runtime parity claim is made for the excluded room's
callbacks. `twisteddimensionmod` and custom `battle.trunc` are excluded from the
fixed `gen9randomdoublesbattle` rules, so their source branches select ordinary
Trick Room and the standard uint32 truncation. The public `Stat` type excludes HP,
matching the source requirement to read maxhp directly. Fractional/unsupported
boost stages cannot enter the typed i8 boost-state contract.
