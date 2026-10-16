# Diagnose ladder recordings

`crates/pyengine/tools/ladder_diag.py` reads the optional `record.dir` output from
`play/recording.py`. It runs offline with the Python standard library and the
repository's `data/training/dex.json`. It does not load a model, import the native
engine, contact Showdown, or change an input file. Run it from the repository root.

```sh
nice -n 10 env PYTHONPATH="$PWD/crates/pyengine" \
  /Users/aminaliu/.t3/worktrees/numbrion-mac/t3code-c1f8987c/.venv/bin/python \
  crates/pyengine/tools/ladder_diag.py \
  /Users/aminaliu/.t3/projects/numbrion-mac/scratch/ladder-diag/records-partial \
  --partial \
  --out scratch/ladder-diag/partial-report.md \
  --cases scratch/ladder-diag/partial-cases.jsonl \
  --summary scratch/ladder-diag/partial-summary.json
```

The required outputs are a Markdown report and one JSON object per extracted
case. `--summary` adds aggregate JSON and a battle inventory. Input arguments may
be room files or directories, scanned recursively for `*.jsonl.gz`. Keep the
source recordings with the case JSONL. Each case includes the original request
and a snapshot reference containing the absolute input path, zero-based decoded
event index, decision sequence, request id, and `analysis.search_record` field.
An exposure with no preceding recorded decision has null request and reference.
The JSONL stores all extracted cases, not just the report's highest ranked cases.

The loader validates each concatenated gzip member. A truncated final member is
ignored with a warning, preserving earlier complete members. Corruption and
invalid JSON also produce warnings. Thus a live copy may be useful but incomplete.
Decisions sent as fallback remain fallbacks. A `late_decision` search is counted
but never presented as the search that selected the action actually sent.

## Classification rules

Sleep exposures include opponent sleep-move uses against our side, including
misses, failures and immunities, our Yawn drowsiness, and our sleep statuses. A
use, its drowsiness and its eventual sleep are grouped into one exposure rather
than counted three times. Sleep without a matching opponent move, including
self-induced Rest, is retained as other. The sleep list includes Spore, Sleep
Powder, Hypnosis, Yawn, Lovely Kiss, Sing, Grass Whistle, Dark Void and Relic Song.

For each exposure the tool takes the latest accepted, non-replacement decision
on the move's turn. For Yawn it also keeps later turn decisions before the
recorded sleep or switch that cleared drowsiness. It matches normalized move
names in the acting foe's slot in every world's full `text`, and matches that
foe's species in the sampled `team`. It does not use the six-entry merged
`analysis.opponent` display. Both world fraction and prior/solved probability mass
are retained. `observed` is read directly, not estimated from `inferred.moves`,
whose shares use a different denominator.

Candidate `prior` is the solver prior from `search_record`, normalized over its
shortlist. JSONL also retains `policy_prior`, the original unnormalized shortlist
entry's probability from `analysis.candidates`. These can differ even at the same
temperature. The matrix order is preserved when attaching per-candidate payoffs.

The exposure labels are mutually exclusive, evaluated in this order:

- Missing snapshot, unresolved foe identity in any world, or a foe that entered
  after the decision: `unavoidable/other`. This label does not assert that the
  position was mechanically unavoidable.
- H2: the move occurs in fewer than 25% of the sampled worlds. Change the threshold
  with `--rare-fraction`. All worlds must have a matching foe set to use this rule.
- H1: set coverage meets the threshold but no opponent pair from the acting foe's
  slot contains that move.
- H3: set coverage meets the threshold and at least one pair contains it. This is
  evidence to investigate weighting and values, not proof that the value head
  mishandled sleep. A low reply mass and a low conditional payoff are different
  explanations. The report shows both, and JSONL keeps every matching reply and
  its per-candidate payoff. Cells with zero simulation count or an originally
  invalid candidate do not contribute conditional payoffs.

Yawn escape decisions are separately labelled other because a new Yawn reply is
not needed for existing drowsiness to cause sleep. Switching can clear drowsiness;
Protect cannot. Grass types resist powder sleep moves, not Yawn or Hypnosis.
Switches to powder or sleep immunity, target Protect, and attacks against the
sleeper are annotated as conditional prevention hints. No damage, speed, Protect
success, redirection, or ability suppression is simulated. An attack is not
claimed to prevent sleep without evidence that it KOs before the sleep move.

The before/after value comparison uses the chosen search q before exposure and
at the first later actionable boundary after the outcome. A terminal win/loss
can supply the after value if no later decision exists. Policy-only values stay
unknown. The displayed `(v + 1) / 2` is an uncalibrated win-probability proxy.
Changes across decisions also include damage, switching and new information;
they are not isolated causal sleep penalties.

The default endgame rule is both sides having at most two alive Pokemon, **or**
at most three alive in total, including the bench. Unrevealed foe Pokemon count
as alive, based on `teamsize` and distinct faints. `--endgame-per-side N` and
`--endgame-total N` change the thresholds; zero disables that branch. Our own
remaining count comes from the request. Forced replacements and refused choices
are excluded from both sleep decisions and endgame cases. Operational counts
include all emitted decisions, with refusal and late counts reported separately.

Stall means at least one chosen slot uses a move in the explicit `STALL` set in
the tool. It includes Protect variants, guards, recovery, Wish, Substitute, Rest,
Endure, and healing moves. Slots are individually labelled stall, attack, switch,
or other using the request and move dex. Mixed attack/stall pairs are counted
separately. The best attacking comparator must contain an attack and no stall.
The q-gap is best stall q minus best attack q. These are alternatives from the
shortlist only, not every legal action. Missing comparisons stay unknown.

Repeated stalls mean stall-containing choices on two or more consecutive turn
boundaries, excluding forced replacements. Close values means an absolute q-gap
of at most 0.02. Confidently losing means chosen q <= -0.8. Next-turn resolution
lines, faints, the next turn decision's value, and terminal result are retained.
The tool lists explicit turn-limit endings, turn 1000 reached, and inactivity
endings separately from ordinary forfeits.

H4 is a scale comparison. For each chosen stall with negative value, estimate
`v * (1 - 0.995**k)` for k = 1, 3, 5, 10 decisions. The magnitude is the improvement
from delaying a negative training target. Flag a nonnegative best-stall q-gap
that is no larger than that magnitude. The signed effect and the chosen-vs-attack
gap are both retained. Search plays the largest solved strategy weight, so its
chosen action need not maximize q. A negative q-gap is therefore retained as
useful evidence but not called a positive stall advantage. Change the assumptions
with `--discount` and `--horizons 1,3,5,10`. This does not remove the discount from
the critic or prove that discount caused a choice.

## Partial run findings, 2026-10-16

The copied snapshot covers **eight battles**, seven completed and one in progress,
with recorded events from 16:11:51 to 16:38:13 Helsinki time. It is an early partial
snapshot of the planned six-hour run, not the full run. Sources were copied only;
Windows processes, stop files and the other checkout were not changed.

- Results: 3 wins, 4 losses, 1 unfinished. Finished-game win rate 42.9%.
- 105 emitted decisions: 96 search, 9 policy, no fallback or default. Search
  disagreed with policy in 27/96 decisions, 28.1%. Five policy decisions had only
  one legal choice; four had no reproducing sampled world.
- Receipt-to-send time: median 2021.6 ms, p95 2508.2 ms, max 2627.4 ms. No missing
  search snapshots, refused choices, late decisions or truncated members in this
  copy. An unfinished battle is not necessarily a truncated file.
- Five Sleep Powder exposures, all in room `2699438650`, with four recorded sleep
  statuses. All five had the move in 100% of sampled sets and in at least one
  shortlisted pair in every world. H1 = 0, H2 = 0, H3 = 5. These exposures are not
  five independent battles.
- On turn 1, Sleep Powder's aggregate prior mass was 0.8743 and solved mass 0.9365.
  It was not yet revealed. The chosen value was +0.1589, and the chosen action's
  payoff conditional on sleep-containing replies was +0.1524. The next value was
  +0.0354, a decrease of 0.0618 in the probability proxy. Search plainly considered
  this threat and weighted it heavily; the head/leaf-state question remains open.
- On turn 5, a missed Sleep Powder had prior mass 0.1567 and solved mass 0.0346.
  That is a weighting example, but a miss does not measure the cost of actual sleep.
- Five endgame decisions, one stall, no consecutive stalls and no H4 flags.
  In room `2699441728`, turn 10, Floatzel chose Protect at q = -0.6104 with solved
  weight 0.9405. The best attacking q was -0.6693, giving a gap of +0.0589. The
  discount magnitudes for k = 1/3/5/10 were 0.0031/0.0091/0.0151/0.0298. This gap
  exceeds all four estimates. Protect blocked Zekrom's Bolt Strike; Floatzel
  survived the turn. This example does not support the small-discount-margin
  explanation. The sample is too small to judge the broader stalling hypothesis.
- No recorded timer endings or turn-limit games. All completed games were under
  20 turns, so longer-game win-rate buckets have no evidence yet.

Generated local artifacts are in `scratch/ladder-diag/partial-report.md`,
`partial-cases.jsonl`, and `partial-summary.json`. They are ignored by git.

## Recorder gaps and suggested changes

The snapshots support H1/H2 and a narrower H3 investigation today. They do not
identify which simulated replies actually put a Pokemon to sleep. `payoffs`
already average chance seeds and include special retry substitution for invalid
rows. `sampled_valid` and `counts` let the tool exclude substituted cells, but
cannot reconstruct leaf status.

Add per-cell, per-seed leaf status, HP, ended/winner and eval-side fields, plus
leaf decision kind, raw critic output and root critic value. This would separate
an optimistic score for an actually slept leaf from a missed/blocked sleep move,
a replacement boundary, or a different leaf state. It would also show lethal
next-hit risk more directly. Capture per-world opponent requests and active party
indices/species to resolve Illusion/Transform and identity changes. The current
join uses live actor slots and sampled species, so unresolved cases stay other.

Record protocol choice acceptance/refusal and reconnect segment ids explicitly
if analysis across long reconnecting games becomes important. Currently the tool
uses request sequence/rqid and refusal events and assumes the raw message stream
is chronological, without deduplicating a full battle-history replay after rejoin.
A saved replay URL would also distinguish an uploaded replay from the conventional
room URL the tool currently emits. None of these suggestions changes the live run.

## Copy the full run after 22:10 Helsinki and analyze it

Run these commands from the implementation checkout after the run has finished.
They copy Windows files to a fresh laptop temporary directory and then to the Mac.
They never create a file or run a mutating command on Windows.

```sh
task_ladder_hop_dir=$(ssh -o BatchMode=yes -p 91 amin@100.105.66.101 \
  'mktemp -d /tmp/numbrion-ladder-diag-full.XXXXXX')
ssh -o BatchMode=yes -p 91 amin@100.105.66.101 \
  "nice -n 10 scp -q -i /home/amin/.ssh/id_ed25519 -r amin@100.96.185.127:C:/Users/aminu/.t3/projects/numbrion-ladderdiag/scratch/ladder/records/spark-record-6h-20261016 $task_ladder_hop_dir/records"
mkdir -p /Users/aminaliu/.t3/projects/numbrion-mac/scratch/ladder-diag/records-full
nice -n 10 scp -q -P 91 -r \
  "amin@100.105.66.101:$task_ladder_hop_dir/records/." \
  /Users/aminaliu/.t3/projects/numbrion-mac/scratch/ladder-diag/records-full/

nice -n 10 env PYTHONPATH="$PWD/crates/pyengine" \
  /Users/aminaliu/.t3/worktrees/numbrion-mac/t3code-c1f8987c/.venv/bin/python \
  crates/pyengine/tools/ladder_diag.py \
  /Users/aminaliu/.t3/projects/numbrion-mac/scratch/ladder-diag/records-full \
  --out scratch/ladder-diag/full-report.md \
  --cases scratch/ladder-diag/full-cases.jsonl \
  --summary scratch/ladder-diag/full-summary.json
```

Do not pass both partial and full copies to the same analysis; that would count
those rooms twice. Retain `--partial` if the source run is still live.

## Tests

```sh
nice -n 10 env PYTHONPATH="$PWD/crates/pyengine" \
  /Users/aminaliu/.t3/worktrees/numbrion-mac/t3code-c1f8987c/.venv/bin/python \
  -m pytest -q crates/pyengine/tools/test_ladder_diag.py
```
