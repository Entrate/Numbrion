# Recovered workflow progress

Pinned Showdown source: `7332b60e22b9e8194bb53549549eba241d73cc9a`.
Original Claude session: `d59f0cff-39d7-4de0-a0b6-97771e8431b8`.
Original workflow: `wf_8d2f29e3-954`.

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
