# Passive offensive ability callback port

Completes the 25 files / 34 function sites assigned to `passive_offense`, using
Showdown `7332b60e22b9e8194bb53549549eba241d73cc9a`. Files cite their pinned
`data/abilities.ts` entries. No payload words, direct PRNG draws or hook waivers.

Ports stat/type/flag multipliers, low-HP thresholds, Technician, Stakeout, status
boosts, Adaptability, Infiltrator, Unseen Fist, Serene Grace and Sheer Force. The
mutable move ports preserve missing/zero chances and secondary-array presence.
Clangorous Soulblaze is outside this format/scope.

Validation: 1,440 pinned oracle vectors cover all 30 numeric callback sites,
incoming modifiers, Technician's 60-power boundary, Adaptability's 2.25 STAB,
healthy/poison/burn/toxic states and physical/special categories. Separate tests
cover four move-mutation callbacks, nested self chances, absent chances, empty
secondaries and Sheer Force's existing boost flag. Registry checks all 34 sites.
Regenerate with `node tools/probes/passive_offense/vectors.mjs [showdown checkout]`.

Full battle acceptance is pending the unfinished core owners. Adaptability's
normal type query retains the existing `has_type` call; vectors use source
`forceSTAB` to exercise the independent branch without replacing that query.
These callback checks do not establish complete engine parity.
