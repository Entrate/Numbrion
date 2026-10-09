# Passive defense batch port

Pinned source: Showdown `7332b60e22b9e8194bb53549549eba241d73cc9a`,
`data/abilities.ts`; baseline `a874a2d`, shared prerequisites `63e7304` and
`77d57e7` (including `4f37a05`). The pinned checkout was read only.

## Implemented coverage

All 109 function hooks in `passive_defense` have bodies, across 59 canonical
effect files: 57 abilities and the distinct Flash Fire/Unburden conditions.
Shell Armor's one constant hook remains in the central manifest dispatcher and
requires no effect file. No callback TODOs, runtime silent stubs, or hook waivers
were added. Each wrapper records exact source callback lines; `sites.json` records
the full hook mapping. Shared private generic bodies live under
`effects/abilities/passivedefense/`, referenced through Aroma Veil's local module.
The hooks preserve undefined, null, false, zero, booleans, numbers and scratch-array
identity rather than collapsing their returns.

The port covers arithmetic and accuracy modifiers; reflected/deleted/negated/sparse
boosts; type conversion and immunity overlays; status and volatile guards; update
cures; ability/condition announcements; absorption callbacks; damage prevention;
PP pressure; No Guard relationships; Unaware's ordered zero assignments; stable
Shield Dust filtering; and Slow Start's counter lifecycle. Hit-data access creates
the slot-keyed object presence exactly when the source queries it. None of these
callbacks draws directly; all nested core calls retain their eventual event draws.

## State and source constraints

Only Slow Start uses custom state: payload word 0 is `counter`, present bit 8
records property presence. Start creates counter 5; eligible residuals decrement;
expiry logs then deletes presence; End observes called-back state and does not
clear the live counter itself. All other files use zero custom payload words.
No scratch handles survive callback scope or enter snapshot payloads.

The fixed format cannot generate Z moves, Max moves or Dynamax. Accordingly the
conversion exclusions' Z/Max branches are unreachable and absorption `baseMaxhp`
equals snapshot `max_hp`. The numeric probe additionally verifies all 359 scoped
moves lack `ohko`; Sturdy's TryHit OHKO branch cannot be reached by a scoped
ActiveMove and explicitly records that constraint. Attract is outside the scoped
condition catalog: Oblivious's source Update branch is still modeled by the generic
host and fails explicitly if an impossible live Attract state reaches production
removal. Scope membership is not synthesized by assigning EffectId::NONE.

## Verification

`cargo test -p engine -j1 --offline`: **55 unit tests + 6 Stage 1 tests + 2 Stage 2A
tests passed**. This includes 10 private passive-defense tests, complete function
manifest coverage, real query integration, and **3,691 pinned callback oracle
rows**:

| Fixture | Rows | Checked behavior |
|---|---:|---|
| numeric.tsv | 2,784 | Relay/modifier arithmetic, fractional modifier state, category/type/contact/sound, full HP/typeMod, status/ability queries, boolean accuracy |
| guards.tsv | 558 | TryHit/status/volatile blocks, exact structural log tokens, boost deletion, sparse insertion order and presence |
| moves.tsv | 108 | Type exclusions, Tera Blast, overlay flags, immunity boolean/object semantics, preexisting type entries |
| absorption.tsv | 36 | Heal/boost/volatile defaulting, accuracy edit order, falsy return variants, immunity logs |
| state.tsv | 13 | Slow Start initial inactive-turn skip, decrement/expiry/delete, restart, called-back End guard |
| mirror.tsv | 12 | Reflection order, stage -6 skip, deletion, exact core attribution, live HP recheck after an earlier callback |
| events.tsv | 108 | No Guard null/undefined move guards, accuracy relay identity, Pressure allies, Unaware sparse assignments, immunity/damage sentinels |
| update.tsv | 72 | Cure/volatile removal/announcement order and successful/irrelevant statuses |

An additional test checks Shield Dust allocates a distinct stable filtered relay
array, retains truthy empty `self` objects, preserves the input array, and releases
both scratch slots. The callback tests assert unchanged `[1,2,3,4]` PRNG where they
execute Battle callbacks. Core-call recording hosts have no RNG API and verify
statement order against actual pinned callbacks; they do not replace production
core methods with no-ops. Fixture scripts are `tools/probes/passive-defense-*.mjs`
and load the pinned build through the repository's commit-checking `loadSim` helper.

## Remaining integration boundaries

This is callback implementation and callback-level parity, **not whole-battle
acceptance**. Production handlers call existing generic core methods for
`heal`, `boost`, `add_volatile`, `remove_volatile`, and `cure_status`; those mutation
pipelines remain explicitly unfinished elsewhere in this baseline. Recording hosts
verify their requested calls and decisions while intentionally excluding internal
HP/status/volatile event mutation, downstream draws and final battle protocol
formatting. Those paths require the respective core ports and the common sample-50
and fuzz replay gates. Slow Start residual ordering, activation timing, move hit
pipeline entry, faint/switch flow, and batch suppression through real complete
battles also remain integration checks. No whole-battle replay or performance claim
is made.

## Canonical source map

| Effect | Function hooks | Callback lines in data/abilities.ts |
|---|---:|---|
| abilities:aromaveil | 1 | 235 |
| abilities:bulletproof | 1 | 481 |
| abilities:clearbody | 1 | 524 |
| abilities:comatose | 2 | 589, 592 |
| abilities:compoundeyes | 1 | 668 |
| abilities:contrary | 1 | 679 |
| abilities:eartheater | 1 | 1134 |
| abilities:filter | 1 | 1285 |
| abilities:flashfire | 2 | 1343, 1352 |
| abilities:flowerveil | 3 | 1421, 1436, 1446 |
| abilities:fluffy | 1 | 1460 |
| abilities:fullmetalbody | 1 | 1560 |
| abilities:furcoat | 1 | 1581 |
| abilities:galvanize | 2 | 1600, 1611 |
| abilities:goodasgold | 1 | 1632 |
| abilities:hustle | 2 | 1912, 1916 |
| abilities:icescales | 1 | 2028 |
| abilities:innerfocus | 2 | 2155, 2158 |
| abilities:insomnia | 3 | 2170, 2176, 2183 |
| abilities:keeneye | 2 | 2262, 2271 |
| abilities:liquidvoice | 1 | 2419 |
| abilities:magicguard | 1 | 2467 |
| abilities:mindseye | 2 | 2621, 2631 |
| abilities:mirrorarmor | 1 | 2659 |
| abilities:moldbreaker | 2 | 2691, 2694 |
| abilities:motordrive | 1 | 2737 |
| abilities:multiscale | 1 | 2762 |
| abilities:noguard | 2 | 2973, 2976 |
| abilities:oblivious | 4 | 3010, 3022, 3025, 3031 |
| abilities:overcoat | 2 | 3110, 3114 |
| abilities:owntempo | 4 | 3146, 3152, 3155, 3160 |
| abilities:pixilate | 2 | 3299, 3310 |
| abilities:pressure | 2 | 3439, 3442 |
| abilities:prismarmor | 1 | 3476 |
| abilities:punkrock | 2 | 3592, 3598 |
| abilities:purifyingsalt | 4 | 3620, 3626, 3633, 3640 |
| abilities:rockhead | 1 | 3908 |
| abilities:sapsipper | 2 | 4026, 4034 |
| abilities:scrappy | 2 | 4082, 4089 |
| abilities:shadowshield | 1 | 4146 |
| abilities:shielddust | 1 | 4231 |
| abilities:slowstart | 5 | 4313, 4319, 4329, 4334, 4339 |
| abilities:solidrock | 1 | 4416 |
| abilities:soundproof | 2 | 4438, 4444 |
| abilities:sturdy | 2 | 4675, 4682 |
| abilities:telepathy | 1 | 4934 |
| abilities:teravolt | 2 | 4981, 4984 |
| abilities:thickfat | 2 | 5018, 5025 |
| abilities:tintedlens | 1 | 5037 |
| abilities:turboblaze | 2 | 5205, 5208 |
| abilities:unaware | 1 | 5217 |
| abilities:unburden | 3 | 5238, 5242, 5245 |
| abilities:vitalspirit | 3 | 5318, 5324, 5331 |
| abilities:voltabsorb | 1 | 5343 |
| abilities:waterabsorb | 1 | 5366 |
| abilities:waterbubble | 6 | 5381, 5387, 5392, 5397, 5402, 5408 |
| abilities:waterveil | 2 | 5432, 5438 |
| conditions:flashfire | 4 | 1357, 1361, 1368, 1374 |
| conditions:unburden | 1 | 5249 |
