# Core ability/item, contact and Pokemon queries

Base: `a874a2d`, branch `finish/core-queries`. Authoritative oracle:
`7332b60e22b9e8194bb53549549eba241d73cc9a` at the read-only pinned checkout.
Root prerequisite commits `63e7304` and `4f37a05` were merged before validation.

## Implemented APIs

- `mutators/items.rs`: `has_ability`, `ignoring_ability`, `has_item`,
  `ignoring_item`. Source: `sim/pokemon.ts:858-889,1895-1902,1952-1959`.
  Raw ID matching precedes suppression, as in the pinned source. Transformation
  suppression precedes `cantsuppress`; gas holders are checked without recursive
  `hasAbility`, preserving inactive/fainted distinctions and `abilityState.ending`.
  Items check Primal Orb before inactivity, then Embargo/Magic Room/Klutz.
- `mutators/pokemon.rs`: `check_move_makes_contact`,
  `check_move_bypasses_protect` (`sim/battle.ts:1293-1313`);
  `get_move_data`, `has_move`, `positive_boosts`, `get_capped_boost`,
  `get_undynamaxed_hp`, `get_nature`
  (`sim/pokemon.ts:690-704,748-752,1201-1219,1614-1623,1965-1967`).
  Contact reads the mutable flag before Protective Pads, with structural
  activation logs. Long Reach acts through its separate ModifyMove callback.
  Protect uses the real HitProtect event with attacker as target and defender as
  source. Sparse boost order and present zero capped results are preserved;
  HP uses JS `amount || hp` truthiness, including zero and NaN.
- `damage/immunity.rs`: `get_types`, `has_type`, `is_grounded`,
  `is_semi_invulnerable`, `effective_weather`
  (`sim/pokemon.ts:1567-1579,2138-2205`).
  Type events preserve input-array mutation and replacement-array ownership;
  empty returned arrays become Normal, added types remain separate, ordinary
  Tera bypasses Type/added types, and Stellar uses the ordinary type path.
  Groundedness preserves repeated Type queries and Levitate's distinct null
  result. Semi-invulnerability reads charge conditions and the opposing Sky Drop
  source relation. Effective weather delegates field suppression to root's helper,
  then applies personal Utility Umbrella and current-effect Mega Sol rules.

Public interface correction approved by root: `check_move_bypasses_protect`
now takes `&mut self` because its source calls `runEvent('HitProtect', ...)`.
No existing callers needed changes. The three `query_has_*` crate-private helpers
use static generated Dex keys and avoid inventing executable IDs for excluded
abilities/items/conditions. None of these helpers allocates or draws directly.

## Verification

The private, checked-in `mutators/queries/oracle.cjs` executes actual pinned
Battle/Pokemon methods. It suspends only the constructor's automatic start and
sets fixture state explicitly; no query/event mechanics are substituted. It
verifies the oracle checkout SHA before extraction and never writes there.
`mutators/queries/vectors.tsv` contains **1,196 comparisons**:

- All 207 executable ability IDs plus empty ability, each active/inactive and
  ordinary/transformed, compare ability/item suppression and both has queries.
- All 61 executable items plus empty item, each active/inactive.
- Type/empty-array Normal fallback, added types, ordinary/Stellar Tera,
  exclude-added and pre-Tera flags, including persisted original-array mutation.
- Mutable contact/protect flags, physical/status category and blockStatus;
  sparse boost caps/order/zero results; HP absent/zero/NaN/negative/fraction/Infinity.
- Levitate vs ordinary ability with active/inactive targets, Flying type,
  negateImmunity, active-move IGNORE_ABILITY and self-user exceptions.
- Empty/sun/rain/sand/snow weather with Cloud Nine holder active/inactive,
  fainted/living and ability-state ENDING true/false.

Additional Rust tests cover Transform's current virtual-slot lookup, disabled and
zero-PP moves still being present, immutable nature, absent named effects never
matching NONE, and insertion/removal of Phantom Force semi-invulnerability.
Every vector asserts unchanged `[1,2,3,4]` PRNG. Type vectors check scratch relay
reclamation. Contact vectors check unchanged logical protocol count.

Commands run with `/bin/bash`, `login=false`, offline Cargo and one Rust job:

```text
cargo check -p engine -j 1 --offline
cargo test -p engine -j 1 --offline query_tests -- --nocapture
cargo test -p engine -j 1 --offline
```

Results: **45 unit tests + 6 Stage 1 + 2 Stage 2A pass**, zero failures/ignored.
This includes root's **12,480** actual pinned singleEvent/runEvent suppression
comparisons. `git diff --check` passes. No end-to-end battle replay is claimed.

## Dependencies and remaining scope

The implementation uses real root-owned active-move accessors, event dispatch,
`present::ENDING`, `suppressing_ability`, and `field_effective_weather`.
Unfinished Type/HitProtect effect callbacks remain reached explicit failures;
no query replaces a missing effect with a no-op. The existing Type scratch/snapshot
capacity remains guarded if a handler expands persistent base types beyond two.

Generated scope excludes Neutralizing Gas, Klutz, Ability Shield, Protective Pads,
Utility Umbrella, Mega Sol, Primal Orbs, and most groundedness suppression conditions.
Their source branches are preserved through static keys/properties, but no runtime
parity claim is made for IDs absent from the executable Dex. No test fabricates IDs
or expands the generated scope to reach them. Current `ignoring_item` matches the
source default `isFling=false`; the true Fling mode differs only for Klutz, excluded
from this scope. Hidden Power text aliases likewise cannot enter the scoped typed
move ID interface. Dynamax/Z/Max moves are excluded by this format: undynamaxed HP
requires no scaling, and protect has no Z/Max bypass bookkeeping overlay.

Other methods in these three files retain their explicit stubs, including
item/ability lifecycle, forme/Transform mutation, and damage/status immunity.
The separate action/mutation owners must implement them before whole-battle
acceptance. This report covers the coherent query/contact slice only.
