# Recovered workflow progress

Pinned Showdown source: `7332b60e22b9e8194bb53549549eba241d73cc9a`.
Original Claude session: `d59f0cff-39d7-4de0-a0b6-97771e8431b8`.
Original workflow: `wf_8d2f29e3-954`.

## GitHub recovery update — 2026-10-09

The older research-workflow checklist below describes the first recovery, not the
current engine milestone. Subsequent work on branch `engine` already committed
the protocol/log-shape specification, oracle fixtures and replay harness,
deterministic generated dex and constructor, event/RNG tracing, and the Stage 2A
compiling API skeleton with the implementation plan and 18 effect batches.

The later module/effect fan-out stopped at the Claude usage limit. All 19 saved
worktrees are clean at `7ff6af5`; there are no additional commits or worktree code
changes to merge. Their core implementation and effect-porting assignments remain
unfinished. Resume from `docs/design/IMPLEMENTATION-PLAN.md` and
`docs/design/EFFECT-BATCHES.json`, using the current branch's API rather than an old
worktree or a hard reset from a saved task prompt.

Recovered and completed in this update:

- Directed-corpus generation and callback coverage tools, with usage documented in
  `docs/design/CORPUS.md`.
- Fixed default-level validation for packed level-100 teams and explicitly allowed
  Levitate's grounding/immunity behavior in expanded batch/effect profiles.
- Validated 16 battles for each of 24 profiles (384 total), including all 18 effect
  batches, both single-effect examples and the four slices. Every fixture rebuilt,
  passed profile constraints and replayed directly and through BattleStream.
  `data/fixtures/directed-smoke.jsonl.gz` preserves this corpus; the report is
  `docs/design/reports/recovery-validation.json`.
- Eight driver-equivalence self-tests passed. Instrumented coverage replay passed
  for the 50 committed random fixtures plus 12 directed fixtures.
- Preserved the original 2,000-battle coverage results (815/852 function callbacks
  and all four constants reached, with no replay differences) in
  `docs/design/reports/oracle-coverage-2000.{json,txt}`.
- Preserved the self-play training notes in `docs/training/TIPS.md`.

Stage 2B event/registry work is still being written by the existing background
Codex task. Preserve its checkpoint on `recovery/stage2b-2026-10-09`; this is
unfinished work, not a claim of event parity or a completed engine. The original
task brief is saved in `docs/design/recovery/STAGE2B-TASK.md`. Do not run another
writer against its paths while it remains active. Battle execution still depends
on the unfinished lifecycle, choices, moves, damage, mutators and log owners.

Local recovery archive:
`/home/aminaliu/.t3/backups/numbrion-fold-20261009T181007Z/`.
It contains all Git refs, the uncommitted files, scratch prompts/logs, thread and
agent histories, the original fuzz corpora and coverage index, and remaining
temporary research scripts. Private transcripts and large scratch corpora stay in
this local archive. T3 Code and the laptop's T3 Connect were left running.

## Completed during recovery

- **spec:state-model** — finished the existing `showdown/04-state-model.md`, reconciled it with the final scope census, corrected EffectState target assumptions and unsafe compact-container suggestions, and added verification notes. `node tools/oracle/check-state-model.mjs /home/aminaliu/src/pokemon-showdown` passes all 25 probes. These are representative oracle checks, not exhaustive validation of the compact Rust layout.
- **scope:extract** — inspected the previously completed run and independently regenerated 50,000 teams. JSON matched the saved artifact exactly after excluding generation timestamp and elapsed time. Counts: 572 species records, 359 moves, 207 abilities, 61 non-empty items and 75 condition records. All declared species/moves/abilities/roles/Tera types were observed; no detected dynamic references are unresolved, no scanned engine sites are unclassified, and no referenced species/move/ability/item is missing. Added explicit catalog limitations to the generator and regenerated the documents. The catalog deliberately includes conservative engine/source-effect references; it is not an exact reachability proof.

## Previously completed authoring

Event-system, turn-loop-and-choices, and move-execution-and-damage specs (01–03). Their separate adversarial reviewers were interrupted; do not infer those reviews completed merely because the authoring did.

## Remaining tasks

- Finish the independent reviewers for event-system, turn-loop, and move-execution.
- Author the protocol/PRNG/team spec (05); its extensive research and working example are saved in `/tmp/psdoc` and the research archive below.
- Run the state-model and protocol/PRNG reviewers, which were never reached in the original pipeline.
- Write the hard-mechanics catalog (07); its scope pipeline stage was never reached.
- Run the completeness critic and write gaps-and-pitfalls (08).

The original workflow journal still reflects the usage-limit failures. It was not edited or given fabricated successful agent results. An unchanged workflow retry will therefore rerun failed tasks, including the authoring recovered here. Consult this tracker and preserve these completed files when continuing; do not blindly replace them with earlier partial output.

## Recovery artifacts

Initial project/source/session backup, including workflow cache and transcripts:
`/home/aminaliu/.t3/backups/numbrion-recovery-20261009T122110Z/progress.tar.gz`.

Original scratch scripts and census outputs (state model, scope and protocol research):
/home/aminaliu/.t3/backups/numbrion-research-20261009T125429Z/research.tar.gz

T3 Code and T3 Connect were left running throughout recovery.
