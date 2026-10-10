# Cross-module integration notes (collected by the integrator)

Expectations that effect batches place on core modules being implemented in parallel. Check each
when the owning module merges; difftest is the final arbiter.

## actions/mutators + damage (owner D, GPT)
- `is_terrain(target)`: Showdown defaults the target to the running event's target, not "field
  terrain for no target" as the skeleton doc says (weather_terrain report).
- `heal(.., source: None, ..)`: None means "omitted" and must default to the event's source
  (weather_terrain).
- `set_weather` / `set_terrain` with `Attribution::DEFAULT` must default source and source effect
  from the active event (weather_terrain).
- `boost`, `is_ally`, `get_side_condition`, `cure_status`: reactive_stats relies on their TS
  semantics incl. omitted-source defaulting (`this.boost(...)` without source = undefined, defaults
  from the event) — see effects-reactive_stats.md.

## actions/moves (owner M) — resolved in fix loop 1
- Resolved: `HitView.callback_effect` distinguishes the live primary move from anonymous nested
  hit objects in both `hit_single_event` and `run_move_effects`. `sourceEffect` remains the live move.
  Dire Claw/Tri Attack now use ordinary source-effect defaulting; the per-effect workaround is removed.
- The mutator defaulting items above are already implemented centrally in `mutation_attribution`,
  `heal`, and `effective_terrain`; their oracle vectors and full-battle scenarios pass.
