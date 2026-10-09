# Contact and status reaction callback port

Completes 13 files / 13 function sites from `reactive_contact` against Showdown
`7332b60e22b9e8194bb53549549eba241d73cc9a`: Aftermath, Cursed Body, Effect Spore,
Electromorphosis, Flame Body, Poison Puppeteer, Poison Touch, Rough Skin, Static,
Synchronize, Toxic Chain, Toxic Debris and Rocky Helmet. Each file cites its
`data/abilities.ts` or `data/items.ts` entry. No own payload words or waivers.

A private, generic host boundary invokes the existing Battle core APIs. The
callback bodies are shared with a test host that records core calls. There is no
trait object, production allocation, alternate RNG or replacement core algorithm.
The ports retain query order, source attribution, damage fractions, ally targeting,
Pads announcement flags and Poison Touch's intentionally reversed contact query.
Max/future moves are absent from the frozen format and scoped move data.

Validation: 34,944 pinned source callback action/PRNG vectors (21 scenarios and
128 seeds per effect) check successful/failed rolls, contact/powder immunity,
Shield Dust/Covert Cloak short circuits, Disable/Struggle guards, ally/self hits,
status exclusions, Pecharunt identity, effect ownership and hazard layers.
Three real Battle adapter tests cover captured state, ally-side selection,
missing layer presence, live PRNG and synthetic source identity. All 13 function
sites pass registry coverage. Regenerate with
`node tools/probes/reactive-vectors.mjs [showdown checkout]`.

Synchronize's `{id: "synchronize", status: status.id}` source object is represented
by `EffectRef::Synchronize(status)`, distinct from the actual ability. Its status
survives EffectToken storage; its absent effectType is `EffectType::Undefined` and
it supplies no ability callbacks. Future set_status must consume `synthetic_status`
for the source's repeated-status/immunity messages (pokemon.ts:1705-1729).

Toxic Debris reserves the integration contract for the pending Toxic Spikes owner:
`layers` is payload word 0 with custom presence bit 8. A missing property compares
as undefined in the source, so an existing layer-less condition does not gain a
layer. The hazard owner must use the same mapping when its callbacks are ported.

Full battle integration remains pending contact/item/ability suppression queries,
status immunity, damage, add_volatile, try_set_status, add_side_condition and text
logging. Action/PRNG vectors validate callback decisions with a recording host;
they do not validate those unfinished mutators or full sample/fuzz battle replay.
