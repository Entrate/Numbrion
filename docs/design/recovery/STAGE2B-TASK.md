Stage 2A is committed (7ff6af5, rustfmt applied to all non-generated files; generated dex modules are
#[rustfmt::skip]). Six Claude Sonnet agents are now implementing the other owners' modules IN PARALLEL in
separate git worktrees: L (src/sim/lifecycle/), C (src/sim/choices/), M (src/actions/moves/),
D split into two agents: D-damage (src/actions/damage/) and D-mutators (src/actions/mutators/), and
T (src/log/text.rs, format.rs, battle.rs). Do NOT edit their paths in this checkout. They report
requested shared-signature changes in docs/design/reports/<owner>.md; I will forward them to you later.

## STAGE 2B — you: the event system + effect registry + effect authoring support

In this checkout (/home/aminaliu/.t3/projects/numbrion, branch engine), implement everything you
assigned to "Lead, 2B" in IMPLEMENTATION-PLAN.md:
1. src/event/: exact comparators and literal selection sort with tie shuffles, stable redirect /
   left-to-right sorts, scratch frames/pins, handler collection for every holder kind and prefix in
   Showdown's order, resolvePriority (incl. SwitchIn special cases, fractional speeds), constants and
   direct calls (call_hook), suppression (ignoringAbility/ignoringItem/suppressingAbility and their
   exceptions), modifier epilogues (chainModify/modify/finalModify relay post-processing),
   singleEvent/runEvent/run_event_spread/priorityEvent/eachEvent/fieldEvent, duration expiry in
   fieldEvent, depth/limit failures, relay semantics (undefined/null/false/0/NotFail/objects).
2. Registry dispatch via build.rs, the manifest coverage check (every function site declared or
   explicitly waived), and a reached-hook coverage report facility (debug feature) for later.
3. docs/design/EFFECTS.md: finalize the authoring guide for many cheaper agents porting effects in
   parallel (precise, example-driven; show how to read args, state, relay, call mutators, log).
   Implement the first vertical-slice effect files as worked examples: the format rules' hooks
   (PotD, Species Clause, HP Percentage Mod, Cancel Mod, Illusion Level Mod, Sleep Clause Mod incl. its
   SetStatus hook), status conditions brn/par/slp/frz/psn/tox, flinch, and Struggle's ModifyMove; then
   confusion. They call D/M/L/T functions that are still todo!() in this checkout — that's expected.
4. Tests: directed Node event probes against the pinned Showdown (outside tools/oracle, e.g.
   tools/probes/event/) comparing handler order, relay values, mutations and PRNG seeds for the
   acceptance cases in your plan row. tools/oracle/trace.mjs + docs/design/TRACE.md (now committed)
   record Showdown's event/handler/RNG trace of real fixture battles; consider emitting the same
   trace format from the engine behind a cargo feature so trace-diff can compare them later — at least
   design the hook points for it in the event dispatcher.

Budget: please be efficient with tokens (the user's GPT quota is limited); prefer targeted reads.
Constraints as before (cargo -j 4 max now — six other agents are building; no network; zero engine
deps; don't touch tools/oracle (except reading), tools/scope, docs/showdown, crates/difftest, the
owners' paths; no git commits). If you must change a shared interface other owners use, keep it
minimal and list every such change precisely in docs/design/reports/lead-2b.md so I can notify them.
Final message: short report (what works, test results, interface changes, what remains).
