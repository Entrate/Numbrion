# Held-item modifier callback port

Completes the 40 files / 73 function sites assigned to `items_modifiers` against
Showdown `7332b60e22b9e8194bb53549549eba241d73cc9a`. Each file cites its pinned
`data/items.ts` entry. No custom payload words, direct RNG draws or hook waivers.

The port covers numeric modifiers, species/form guards, TakeItem restrictions,
Choice lock start/move hooks, Assault Vest move disabling, Life Orb recoil,
Clear Amulet's sparse boost deletion/log conditions and Covert Cloak's new filtered
secondary array. Dynamax, Me First and Octolock are outside the frozen format and
scope; their unreachable branches are identified beside the ports.

Validation: 23,218 pinned callback vectors check 63 numeric/TakeItem function sites
across 13 species, every scoped move type, missing/present sources, two incoming
modifiers and numeric/always-hit accuracy. Separate tests check sparse boost
presence/order, self drops, secondary-presence logging and filtered array identity.
All 73 function sites pass registry coverage. All-features engine tests pass.
Regenerate vectors with `node tools/probes/items-vectors.mjs [showdown checkout]`.

Core integration remains pending: Choice hooks require add/remove_volatile;
Assault Vest requires disable_move; Life Orb requires damage. These calls retain
source semantics and reach the existing explicit core stubs. Full sample/fuzz
battle replay cannot pass until lifecycle/move/damage/mutator/log owners finish.
Registry coverage and isolated callback checks do not imply battle parity.
