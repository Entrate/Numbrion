# Reactive stat callbacks

Completes the 27 canonical ability files and all 39 function sites in
`reactive_stats`, against immutable Showdown commit
`7332b60e22b9e8194bb53549549eba241d73cc9a`. Each callback cites its precise
`data/abilities.ts` line. No waivers, missing hook bodies, production allocations,
trait objects, independent RNG, shared registry changes or alternate mutators.

The private `abilities/reactivestats/host.rs` statically routes callbacks to
existing Battle APIs. A recording host runs the same bodies against pinned JS
callback traces. Boosts retain sparse insertion order, target/source defaults,
explicit effect attribution and secondary/self flags. One-shot Battle Bond,
Dauntless Shield and Intrepid Sword markers use existing Pokemon flags and
preserve assignment order around the boost call. Ruin retains holder identity on
the live move: defensive Ruin rechecks the stored holder's current ability;
offensive Ruin tests only stored-holder presence. Thermal Exchange inspects the
source object's primary status, including Synchronize's anonymous status object.

Berserk owns one payload word: word 0 stores `checkedBerserk` as boolean, custom
presence bit 8 records property creation. Missing, false and true remain distinct.
Damage writes the check before item consumption; AfterMoveSecondary writes true
before its early guards. The production adapter distinguishes missing multihit,
numeric zero, fixed counts and a truthy range object. The common attack-history
reader supplies the latest damage, with totalDamage selected only for multihit
without smartTarget. All other abilities own zero payload words.

Validation:

- 44,109 pinned JS action/state/return vectors: 1,131 profiles at each of the 39
  callbacks. The profile includes individual boundary changes plus deterministic
  combinations of source/holder identities, HP crossings, multihit/smartTarget,
  sparse boost drops, prior flags, absent payload, ability suppression, foe counts,
  ties/zero defense totals, Substitute, Tailwind, status/type/contact guards and
  rejected boost sentinels. The fixture's deterministic profile generator is
  independent of the battle RNG; these callbacks have no direct random draws.
- 13,572 real registry Ruin invocations match the oracle's modifier decision and
  exact stored holder, at three initial modifiers. Seeds remain unchanged.
- Real Berserk dispatch checks absent/false/true payloads, numeric-zero versus
  range multihit, nonhealing items and retired captured listener state. Real
  Thermal Exchange dispatch checks structural From tags, primary/absent/synthetic
  source status, logical line counts and unchanged seeds. Rattled real dispatch
  checks that a missing Attack key stays absent even with nonzero backing storage.
- Intimidate and Download adapter checks exercise real foe enumeration with
  fainted slots, Substitute immunity ordering and zero stat totals. Successful
  boost mutations remain outside those checks while the shared boost API is
  unfinished.

Regenerate fixtures with
`node tools/probes/reactive-stats-vectors.mjs /home/aminaliu/src/pokemon-showdown`.
Fixtures live in the private `reactivestats/vectors/` directory, one file per
ability; no external dependency or network access is needed for Cargo tests.

Dependencies: shared event suppression, active move access, has_ability,
contact, volatile lookup, foe traversal, get_stat, boost and cure_status.
Queries are used through their real APIs; the recording host preserves exact
nested call order without replacing core algorithms. All output is structural.
The source debug statements are disabled in the frozen format (`debugMode` is
false). Water Shuriken and Greninja-Ash are absent from the closed scoped Dex;
Battle Bond's complete ModifyMove conditional is still implemented and tested
with the private reference host, with no runtime waiver or silent reached stub.

Callback coverage and numeric/structural adapter coverage do not establish full
battle parity. The unfinished boost and status mutations, lifecycle/move actions,
text output and common sample-50/fuzz replay remain integration gates.
