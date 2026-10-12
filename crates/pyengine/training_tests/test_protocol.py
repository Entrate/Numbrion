"""Tests for training.protocol: request-only masks and choice text, the own team from the first request, and
server lines in engine form.

Every check compares the protocol-only answer with the engine's own (``Battle.legal_mask``, ``Battle.action_text``,
``Battle.teams``, ``Battle.drain_player_log``). The default run is small; ``NUMBRION_TEST_BATTLES=N`` scales the
random-battle sweeps (battles per pool) and the team checks (teams per pool), and ``pytest -s`` prints the counts.
The simulator comparison needs node and a built Showdown checkout at the oracle commit (``NUMBRION_SHOWDOWN``, or
``scratch/showdown-play``); recorded server streams are read from ``scratch/showdown-logs`` when present.
"""
import gzip
import json
import os
import shutil
import subprocess
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest

import numbrion as nb
from training import protocol as P

REPO = Path(__file__).resolve().parents[3]
POOL_400 = REPO / "crates" / "pyengine" / "testdata" / "pool-400.txt.gz"
POOL_200K = REPO / "data" / "teams" / "pool-s1-200k.txt.gz"
TRAIN_POOL = REPO / "data" / "teams" / "train-s42-2000.txt"
LOGS = REPO / "scratch" / "showdown-logs"


def n_battles(default: int, divisor: int = 1) -> int:
    """``default``, or ``NUMBRION_TEST_BATTLES // divisor`` (at least ``default``) when the variable is set."""
    scale = os.environ.get("NUMBRION_TEST_BATTLES")
    return default if scale is None else max(default, int(scale) // divisor)


def read_pool(path: Path, limit: int | None = None) -> list[str]:
    opener = gzip.open if path.suffix == ".gz" else open
    out = []
    with opener(path, "rt", encoding="utf-8") as f:
        for line in f:
            if limit is not None and len(out) >= limit:
                break
            if line.strip():
                out.append(line.rstrip("\n"))
    return out


@pytest.fixture(scope="module")
def pool400():
    return read_pool(POOL_400)


def test_action_constants_match_the_engine():
    assert (P.N_ACTIONS, P.SWITCH_BASE, P.PASS, P.FORCED_MOVE, P.TARGET_AUTO) == (
        nb.N_ACTIONS, nb.SWITCH_BASE, nb.PASS, nb.FORCED_MOVE, nb.TARGET_AUTO)
    for slot in range(4):
        for loc in range(-2, 3):
            for tera in (False, True):
                assert P.move_code(slot, loc, tera) == nb.move_action(slot, loc, tera)


# ---- masks against the engine ----------------------------------------------------------------------------------

def classify(request: str, engine_joint: np.ndarray) -> str:
    """How the request mask relates to the engine's: "equal", or the hidden-information reasons that explain every
    difference ("maybeTrapped", "maybeDisabled", "struggle"), or "UNEXPLAINED ..."."""
    options = P.request_options(request)
    mask = options.joint() if options is not None else np.zeros_like(engine_joint)
    if np.array_equal(mask, engine_joint):
        return "equal" if engine_joint.any() else "no choice"
    if options is None:
        return "UNEXPLAINED: the request asks for no choice"
    reasons = set()
    ours, theirs = [mask.any(1), mask.any(0)], [engine_joint.any(1), engine_joint.any(0)]
    for k, slot in enumerate(options.slots):
        request_codes, engine_codes = set(np.flatnonzero(ours[k])), set(np.flatnonzero(theirs[k]))
        struggle = slot.maybe_disabled and not slot.forced and {c for c in engine_codes if c < 40} == {P.FORCED_MOVE}
        if struggle:
            reasons.add("struggle")  # Imprison covers every move: the engine plays Struggle
        if engine_codes - request_codes - ({P.FORCED_MOVE} if struggle else set()):
            return f"UNEXPLAINED: slot {k} engine-only codes {sorted(engine_codes - request_codes)}"
        for c in request_codes - engine_codes:
            if P.SWITCH_BASE <= c < P.PASS and slot.maybe_trapped:
                reasons.add("maybeTrapped")
            elif c < P.SWITCH_BASE and slot.maybe_disabled:
                reasons.add("struggle" if struggle else "maybeDisabled")
            else:
                return f"UNEXPLAINED: slot {k} request-only code {c}"
    rows, cols = ours[0] & theirs[0], ours[1] & theirs[1]
    if not np.array_equal(mask[np.ix_(rows, cols)], engine_joint[np.ix_(rows, cols)]):
        return "UNEXPLAINED: joint constraints differ"
    return "+".join(sorted(reasons))


def state(battle) -> tuple:
    return (battle.request_json(0), battle.request_json(1), battle.turn, battle.prng_seed, battle.ended,
            battle.drain_log())


def retry_until_exact(battle, side: int, request: str, engine_joint: np.ndarray) -> int:
    """Play the client: submit a pair the request allows but the engine refuses, read the re-sent request from the
    ``[Unavailable choice]`` rejection, and repeat until the request mask is exact. Returns the number of retries.

    When Imprison covers every move of a slot, the server accepts any move there and plays Struggle instead: then the
    accepted choice must have the same effect as the engine's pair with code 4 in that slot."""
    for retries in range(12):
        mask = P.request_mask(request)[1]
        if np.array_equal(mask, engine_joint):
            return retries
        a0, a1 = (int(x) for x in np.argwhere(mask & ~engine_joint)[0])
        probe = battle.clone()
        try:
            probe.choose(side, P.choice_text(request, a0, a1))
        except nb.ChoiceError as info:
            assert info.args[0].startswith("[Unavailable choice]"), info.args
            assert len(info.args) > 1 and info.args[1], "the rejection re-sends the request"
            battle, request = probe, info.args[1]
            assert np.array_equal(battle.legal_mask(side)[1], engine_joint)
            continue
        options = P.request_options(request)
        codes = [a0, a1]
        for k in range(2):
            moves = {c for c in np.flatnonzero(engine_joint.any(1 - k)) if c < P.SWITCH_BASE}
            if options.slots[k].maybe_disabled and moves == {P.FORCED_MOVE}:
                codes[k] = P.FORCED_MOVE
        assert codes != [a0, a1] and engine_joint[codes[0], codes[1]], "accepted only as Struggle"
        twin = battle.clone()
        twin.choose_action(side, *codes)
        for b in (probe, twin):
            if b.needs_action(1 - side):
                m0, joint = b.legal_mask(1 - side)
                b0 = int(np.flatnonzero(m0)[0])
                b.choose_action(1 - side, b0, int(np.flatnonzero(joint[b0])[0]))
        assert state(probe) == state(twin)
        return retries
    raise AssertionError("the request mask never became exact")


def check_text(battle, side: int, request: str, engine_joint: np.ndarray, rng, stats: Counter, pairs: int) -> None:
    """``choice_text`` equals ``Battle.action_text`` and ``codes_from_choice`` inverts it, on sampled pairs legal for
    both (under an Imprison that covers every move the engine's code 4 = Struggle has no request counterpart)."""
    legal = np.argwhere(engine_joint & P.request_mask(request)[1])
    if len(legal) == 0:
        return
    picks = legal if len(legal) <= pairs else legal[rng.choice(len(legal), pairs, replace=False)]
    parsed = P.load_request(request)
    for a0, a1 in picks:
        a0, a1 = int(a0), int(a1)
        text = P.choice_text(parsed, a0, a1)
        assert text == battle.action_text(side, a0, a1), (request, a0, a1)
        assert P.codes_from_choice(parsed, text) == (a0, a1), (text, a0, a1)
        stats["text pairs"] += 1


def steer(request: str, mask0: np.ndarray, joint: np.ndarray, rng) -> tuple[int, int] | None:
    """Prefer Revival Blessing when it is legal, so replacement requests with ``reviving`` slots occur."""
    parsed = P.load_request(request)
    for k, entry in enumerate(parsed.get("active") or []):
        for j, move in enumerate(entry.get("moves", [])):
            if move.get("id") != "revivalblessing":
                continue
            code = P.move_code(j, 0, False)
            if k == 0 and mask0[code]:
                return code, int(rng.choice(np.flatnonzero(joint[code])))
            if k == 1:
                rows = np.flatnonzero(joint[:, code])
                if len(rows):
                    return int(rng.choice(rows)), code
    return None


def check_lines(normalizer, lines: list[str], stats: Counter, failures: list) -> None:
    """Engine player-view lines pass through the normalizer unchanged."""
    if normalizer(lines) != lines:
        failures.append(("normalize changed engine lines", lines))
    stats["log lines"] += len(lines)


def sweep(pools: list[Path], battles: int, seed: int, stats: Counter, text_pairs: int = 6, n_envs: int = 16):
    """Random self-play in a BatchEnv; at every acting side compare the request mask with the engine's joint mask."""
    env = nb.BatchEnv(n_envs, [str(p) for p in pools], seed=seed, threads=2, joint_mask=True, log=True)
    rng = np.random.default_rng(seed)
    result = env.reset()
    failures = []
    normalizers = {}  # (env, side) -> (battle id, LineNormalizer): engine lines must pass through unchanged
    while stats["battles"] < battles:
        uniform = nb.sample_uniform(result, env, rng)
        actions = env.random_actions(switch_prob=0.15, tera_prob=0.2) if rng.random() < 0.5 else uniform
        for e in range(n_envs):
            battle = None
            for side in range(2):
                observed = env.observe(e, side)
                battle_id, normalizer = normalizers.get((e, side), (None, None))
                if battle_id != observed["battle_id"]:
                    if normalizer is not None:  # the unread end of the battle that just finished
                        check_lines(normalizer, observed["prev_log"], stats, failures)
                    normalizer = P.LineNormalizer()
                    normalizers[(e, side)] = (observed["battle_id"], normalizer)
                check_lines(normalizer, observed["log"], stats, failures)
                if not result["needs_action"][e, side]:
                    continue
                request = observed["request"]
                joint = result["joint"][e, side]
                parsed = json.loads(request)
                stats["boundaries"] += 1
                stats["replacement" if "forceSwitch" in parsed else "move request"] += 1
                if any(m.get("reviving") for m in parsed["side"]["pokemon"]) and "forceSwitch" in parsed:
                    stats["revival"] += 1
                for entry in parsed.get("active") or []:
                    for flag in ("maybeTrapped", "maybeDisabled", "trapped", "canTerastallize"):
                        stats[f"flag {flag}"] += bool(entry.get(flag))
                options = P.request_options(parsed)
                stats["forced move"] += sum(slot.forced for slot in options.slots)
                stats["forced pass"] += options.forced_passes > 0
                stats["auto-pass slot in move request"] += options.kind == "move" and any(s.auto for s in options.slots)
                kind = classify(request, joint)
                stats[kind] += 1
                if kind.startswith("UNEXPLAINED"):
                    failures.append((kind, request))
                    continue
                if not np.array_equal(P.request_mask(request)[0], result["mask0"][e, side]) and kind == "equal":
                    failures.append(("mask0 differs", request))
                if battle is None:
                    battle = env.clone_battle(e)
                check_text(battle, side, request, joint, rng, stats, text_pairs)
                if kind not in ("equal", "no choice"):
                    stats["retries"] += retry_until_exact(env.clone_battle(e), side, request, joint)
                steered = steer(request, result["mask0"][e, side], joint, rng) if rng.random() < 0.8 else None
                if steered:
                    actions[e, side] = steered
        result = env.step(actions)
        stats["battles"] += int(result["done"].sum())
    assert not failures, failures[:3]


@pytest.mark.parametrize("pool", ["pool-400", "pool-s1-200k"])
def test_request_mask_equals_the_engine_mask_except_hidden_information(pool):
    path = POOL_400 if pool == "pool-400" else POOL_200K
    stats = Counter()
    sweep([path], n_battles(60), seed=11 if pool == "pool-400" else 12, stats=stats)
    print(f"\n{pool}: {dict(sorted(stats.items()))}")
    assert stats["boundaries"] > 0 and stats["equal"] > 0.95 * stats["boundaries"]
    assert stats["replacement"] > 0 and stats["text pairs"] > 0


def directed_battle(p1: list[str], p2: list[str], seed=(1, 2, 3, 4)):
    b = nb.Battle(seed, "]".join(p1), "]".join(p2), names=("Alice", "Bob"), log=True)
    b.start()
    return b


MON = "{name}||Leftovers|{ability}|{moves}||85,85,85,85,85,85|{gender}|||80|,,,,,Normal"


def mon(name, moves, ability, gender="M"):
    return MON.format(name=name, moves=",".join(moves), ability=ability, gender=gender)


FILLER = [mon("Snorlax", ["bodyslam", "protect", "curse", "rest"], "Thick Fat"),
          mon("Blissey", ["hypervoice", "protect", "softboiled", "toxic"], "Healer", "F"),
          mon("Skarmory", ["bravebird", "protect", "roost", "spikes"], "Sturdy"),
          mon("Garchomp", ["earthquake", "protect", "dragonclaw", "swordsdance"], "Rough Skin")]
PIKACHU = mon("Pikachu", ["fakeout", "protect", "thunderbolt", "volttackle"], "Lightning Rod")
PARTNER = mon("Indeedee", ["protect", "helpinghand", "shadowball", "encore"], "Psychic Surge", "F")


def test_imprison_and_shadow_tag_are_the_only_differences_and_a_retry_resolves_them():
    """Directed positions: Imprison hides the disabled moves of the last active Pokemon (all of them: Struggle), and
    Shadow Tag hides its trapping behind maybeTrapped. The request mask is a superset; retries make it exact."""
    imprisoner = mon("Mew", ["imprison", "protect", "psychic", "hypervoice"], "Synchronize", "N")
    victim = mon("Farigiraf", ["protect", "psychic", "hypervoice"], "Armor Tail")  # every move imprisoned
    partial = mon("Gengar", ["protect", "shadowball", "sludgebomb", "encore"], "Cursed Body")
    for p1_lead, expected in ((victim, "struggle"), (partial, "maybeDisabled")):
        b = directed_battle([PIKACHU, p1_lead] + FILLER, [imprisoner, PARTNER] + FILLER)
        b.choose(0, "move 2, move 1")  # Protect, Protect
        b.choose(1, "move 1, move 1")  # Imprison, Protect
        request = b.request_json(0)
        assert json.loads(request)["active"][1].get("maybeDisabled")
        joint = b.legal_mask(0)[1]
        assert classify(request, joint) == expected
        # Struggle: the first attempt is accepted and plays Struggle; one disabled move: one rejection reveals it.
        assert retry_until_exact(b.clone(), 0, request, joint) == (0 if expected == "struggle" else 1)
    tagger = mon("Gothitelle", ["fakeout", "protect", "psychic", "trickroom"], "Shadow Tag", "F")
    raichu = mon("Raichu", ["protect", "thunderbolt", "nastyplot", "encore"], "Lightning Rod", "F")
    b = directed_battle([PIKACHU, raichu] + FILLER, [tagger, PARTNER] + FILLER)
    request = b.request_json(0)
    entries = json.loads(request)["active"]
    assert entries[0].get("trapped") and entries[1].get("maybeTrapped")  # slot a's trap is revealed, b's is not
    joint = b.legal_mask(0)[1]
    assert classify(request, joint) == "maybeTrapped"
    assert retry_until_exact(b.clone(), 0, request, joint) == 1


def test_no_choice_requests_and_choice_parsing():
    teams = read_pool(TRAIN_POOL, 2)
    b = nb.Battle((5, 6, 7, 8), teams[0], teams[1], names=("Alice", "Bob"), log=True)
    b.start()
    for wait in ('{"wait":true,"side":{"name":"Alice","id":"p1","pokemon":[]}}', None, "null",
                 '{"teamPreview":true,"side":{"name":"A","id":"p1","pokemon":[]}}'):
        m0, joint = P.request_mask(wait)
        assert not m0.any() and not joint.any() and P.choice_text(wait, 0, 0) is None
    request = b.request_json(0)
    moves = json.loads(request)["active"]
    m0, joint = b.legal_mask(0)
    a0 = next(int(c) for c in np.flatnonzero(m0) if c < 40)
    a1 = next(int(c) for c in np.flatnonzero(joint[a0]) if c >= 40)
    text = b.action_text(0, a0, a1)
    assert P.codes_from_choice(request, f"/choose {text}|7") == (a0, a1)
    name = moves[0]["moves"][a0 // 10]["move"]
    by_name = text.replace(f"move {a0 // 10 + 1}", f"move {name}", 1)
    assert P.codes_from_choice(request, by_name) == (a0, a1)
    with pytest.raises(ValueError):
        P.codes_from_choice(request, "default")
    with pytest.raises(ValueError):
        P.codes_from_choice(request, "pass, pass")


# ---- the own team ----------------------------------------------------------------------------------------------

def team_cases(teams: list[str], seed: int):
    """(side, first request, lines before it, true packed team) of battles between consecutive pool teams."""
    for i in range(len(teams) - 1):
        b = nb.Battle((seed, i % 65536, 3, 4), teams[i], teams[i + 1], names=("Alice", "Bob"), log=True)
        b.start()
        for side in (0, 1):
            yield side, b.request_json(side), b.drain_player_log(side), b.teams[side]


@pytest.mark.parametrize("pool", ["pool-400", "pool-s1-200k", "train-s42-2000"])
def test_team_from_first_request_equals_the_packed_team(pool):
    """Exact on every field, with and without the start lines (consumed items, Imposter, battle-start formes)."""
    path = {"pool-400": POOL_400, "pool-s1-200k": POOL_200K, "train-s42-2000": TRAIN_POOL}[pool]
    count = n_battles(400) if pool != "pool-400" else 400
    teams = read_pool(path, count + 1)
    checked = Counter()
    for side, request, lines, truth in team_cases(teams, seed=len(pool)):
        assert P.team_from_request(request, lines) == truth
        assert P.team_from_request(request) == truth
        checked["teams"] += 1
        if any(line.startswith(("|-enditem|", "|-transform|", "|detailschange|", "|-formechange|"))
               and line.split("|")[2][:2] == f"p{side + 1}" for line in lines):
            checked["changed before turn 1"] += 1
    print(f"\n{pool}: {dict(checked)}")
    assert checked["teams"] >= 2 * (len(teams) - 1)


def test_reconstructed_team_drives_the_world_builder(pool400):
    """A WorldBuilder built from the reconstructed team (not Battle.teams) and fed only protocol inputs reproduces
    the viewer's requests: worlds exist at the first decision and at most later ones."""
    if not nb.SEARCH:
        pytest.skip("wheel built without the search feature")
    rng = np.random.default_rng(7)
    set_pool = nb.SetPool([str(POOL_400)])
    decisions = with_worlds = 0
    for n in range(n_battles(3, 200)):
        viewer = n % 2
        b = nb.Battle((n, 9, 9, 9), pool400[2 * n], pool400[2 * n + 1], names=("Alice", "Bob"), log=True)
        b.start()
        lines = b.drain_player_log(viewer)
        team = P.team_from_request(b.request_json(viewer), lines)
        assert team == b.teams[viewer]
        wb = nb.WorldBuilder(viewer, team, set_pool, names=("Alice", "Bob"), seed=n)
        wb.push_lines(P.normalize_lines(lines))
        while not b.ended and b.turn <= 8:
            if b.needs_action(viewer):
                request = b.request_json(viewer)
                wb.push_request(request)
                decisions += 1
                try:
                    wb.worlds(2, seed=n)
                    with_worlds += 1
                except RuntimeError:
                    assert decisions > 1, wb.last_failure  # the first decision checks the team itself
            for side in (viewer, 1 - viewer):
                if b.needs_action(side):
                    m0, joint = b.legal_mask(side)
                    a0 = int(rng.choice(np.flatnonzero(m0)))
                    a1 = int(rng.choice(np.flatnonzero(joint[a0])))
                    if side == viewer:
                        wb.push_choice(P.choice_text(request, a0, a1))
                    b.choose_action(side, a0, a1)
            wb.push_lines(P.normalize_lines(b.drain_player_log(viewer)))
    print(f"\nworld builder: {with_worlds}/{decisions} decisions with worlds")
    assert with_worlds >= 0.8 * decisions


# ---- server lines ----------------------------------------------------------------------------------------------

def server_stream(lines: list[str], side: int, names=("Alice", "Bob")) -> list[str]:
    """What a websocket client of a Showdown server receives for the engine's player-view ``lines``: room headers,
    the join scrollback, timestamps, avatars, room lines, chat, timer notes and requests mixed in."""
    out = [">battle-gen9randomdoublesbattle-1", "|init|battle", f"|title|{names[0]} vs. {names[1]}", "|j|☆Alice",
           f"|player|p1|{names[0]}|102|", f"|player|p2|{names[1]}|2|", "|:|1791787775", "", "|J|☆Bob",
           "|c|☆Bob|gl hf", "|c:|1791787776|☆Alice|you too", "|raw|<div class=\"broadcast-blue\">hi</div>",
           "|inactive|Battle timer is ON: inactive players will automatically lose when time's up.",
           "|uhtml|invites|<div>x</div>", "|rated|", "|html|<b>x</b>"]
    for i, line in enumerate(lines):
        if line == "|t:|":
            line = f"|t:|{1791787775 + i}"
        elif line.startswith("|player|"):
            fields = line.split("|")
            line = f"|player|{fields[2]}|{fields[3]}|{['102', '2'][fields[2] == 'p2']}|1500"
        elif line.startswith("|turn|"):
            out += [">battle-gen9randomdoublesbattle-1", f'|request|{{"wait":true,"side":{{"id":"p{side + 1}"}}}}',
                    "|inactive|Alice has 150 seconds left.", f"||{names[0]} | Time left: 150 sec this turn",
                    "|error|[Invalid choice] Can't do anything: It's not your turn", "|player|p2|Bob|2|",
                    "|l|☆Bob", "|n|☆Bob2|bob"]
        out.append(line)
    out += ["|raw|Alice's rating: 1000 &rarr; <strong>1020</strong>", "|-message|Bob forfeited.",
            "|-hint|If you want to tie earlier, consider using `/offertie`.", "|inactiveoff|Battle timer is now OFF.",
            "|bigerror|The simulator process crashed. We've been notified and will fix this ASAP.", ""]
    return out


def test_normalize_lines_recovers_the_engine_stream(pool400):
    rng = np.random.default_rng(3)
    for n in range(n_battles(4, 100)):
        b = nb.Battle((n, 1, 2, 3), pool400[n], pool400[n + 1], names=("Alice", "Bob"), log=True)
        b.start()
        streams = [b.drain_player_log(0), b.drain_player_log(1)]
        while not b.ended:
            for side in (0, 1):
                if b.needs_action(side):
                    m0, joint = b.legal_mask(side)
                    a0 = int(rng.choice(np.flatnonzero(m0)))
                    b.choose_action(side, a0, int(rng.choice(np.flatnonzero(joint[a0]))))
            for side in (0, 1):
                streams[side] += b.drain_player_log(side)
        for side in (0, 1):
            assert P.normalize_lines(server_stream(streams[side], side)) == streams[side]
            normalizer = P.LineNormalizer()  # fed one server message at a time
            fed = []
            for line in server_stream(streams[side], side):
                fed += normalizer([line])
            assert fed == streams[side]


SIM_SCRIPT = r"""
const [root] = process.argv.slice(1);
const sim = require(root + '/dist/sim');
const {extractChannelMessages} = require(root + '/dist/sim/battle');
const input = JSON.parse(require('fs').readFileSync(0, 'utf8'));
const out = {p1: [], p2: []};
const updates = [];
const battle = new sim.Battle({formatid: 'gen9randomdoublesbattle', seed: input.seed,
                               send: (type, data) => { if (type === 'update') updates.push(data.join('\n')); }});
const flush = () => {
  battle.sendUpdates();
  for (const text of updates.splice(0)) {
    const channels = extractChannelMessages(text, [1, 2]);
    out.p1.push(...channels[1]);
    out.p2.push(...channels[2]);
  }
};
flush();
battle.setPlayer('p1', {name: input.names[0], team: input.teams[0]}); flush();
battle.setPlayer('p2', {name: input.names[1], team: input.teams[1]}); flush();
for (const [side, choice] of input.choices) {
  if (!battle.choose(side, choice)) throw new Error('rejected ' + side + ' ' + choice);
  flush();
}
process.stdout.write(JSON.stringify(out));
"""


def showdown_checkout() -> Path | None:
    candidates = [os.environ.get("NUMBRION_SHOWDOWN"), REPO / "scratch" / "showdown-play"]
    commit = (REPO / "tools" / "oracle" / "ORACLE_COMMIT").read_text().strip()
    for c in candidates:
        if not c or not (Path(c) / "dist" / "sim" / "battle.js").exists():
            continue
        head = subprocess.run(["git", "-C", str(c), "rev-parse", "HEAD"], capture_output=True, text=True).stdout
        if head.strip() == commit:
            return Path(c)
    return None


def test_simulator_player_streams_normalize_to_the_engine_streams(pool400):
    """The same battle (seed, teams, names, choices) in the pinned Showdown simulator: each player's channel, the
    lines a server forwards to that player, equals the engine's player view once normalized."""
    root = showdown_checkout()
    if root is None or shutil.which("node") is None:
        pytest.skip("needs node and a built Showdown checkout at the oracle commit")
    rng = np.random.default_rng(5)
    for n in range(n_battles(3, 200)):
        names = ("Alice Smith", "Bob")
        seed = (n + 1, 2, 3, 4)
        b = nb.Battle(seed, pool400[10 + n], pool400[20 + n], names=names, log=True)
        b.start()
        streams = [b.drain_player_log(0), b.drain_player_log(1)]
        choices = []
        while not b.ended:
            for side in (0, 1):
                if b.needs_action(side):
                    m0, joint = b.legal_mask(side)
                    a0 = int(rng.choice(np.flatnonzero(m0)))
                    a1 = int(rng.choice(np.flatnonzero(joint[a0])))
                    choices.append([f"p{side + 1}", P.choice_text(b.request_json(side), a0, a1)])
                    b.choose_action(side, a0, a1)
            for side in (0, 1):
                streams[side] += b.drain_player_log(side)
        payload = json.dumps({"seed": ",".join(map(str, seed)), "names": names, "teams": list(b.teams),
                              "choices": choices})
        run = subprocess.run(["node", "-e", SIM_SCRIPT, str(root)], input=payload, capture_output=True, text=True,
                             timeout=120)
        assert run.returncode == 0, run.stderr[-2000:]
        sim = json.loads(run.stdout)
        for side in (0, 1):
            raw = sim[f"p{side + 1}"]
            assert any(line.startswith("|t:|1") for line in raw)
            assert P.normalize_lines(raw) == streams[side]
            assert P.normalize_lines(server_stream(raw, side, names)) == streams[side]


def recorded_battles():
    return sorted(LOGS.glob("*.json")) if LOGS.is_dir() else []


def replay_recording(record: dict, set_pool, check_every: int, stats: Counter) -> None:
    """Feed a recorded server stream to a WorldBuilder the way a bot would, from protocol inputs only: normalized
    lines, the requests (the pinned server sends a request right after the lines that lead to it) and the choices
    the bot sent (the last one per rqid was accepted). Worlds must reproduce the requests."""
    side = int(record["side"][1]) - 1
    errors = [m["time"] for m in record["messages"] if "|error|" in m["text"]]
    items = sorted(record["sent"], key=lambda item: item["time"])
    sent = {}
    for i, item in enumerate(items):
        until = items[i + 1]["time"] if i + 1 < len(items) else float("inf")
        if any(item["time"] <= t < until for t in errors):
            continue  # refused by the server
        body, _, rqid = item["text"].split("|", 1)[1].rpartition("|")
        if body.startswith("/choose ") and rqid.isdigit():
            sent[int(rqid)] = body[len("/choose "):]
    normalizer = P.LineNormalizer()
    wb = team = pending = None
    start_lines, decisions = [], 0
    for message in record["messages"]:
        raw = message["text"].split("\n")
        requests = [line[len("|request|"):] for line in raw if line.startswith("|request|")]
        lines = normalizer(raw)
        if lines and pending is not None:
            wb.push_request(pending[0])
            wb.push_choice(pending[1])
            pending = None
        if wb is None:
            start_lines += lines
        else:
            wb.push_lines(lines)
        for text in requests:
            request = json.loads(text)
            if P.request_options(request) is None:
                continue
            if wb is None:
                if sum(line.startswith("|turn|") for line in start_lines) != 1:
                    stats["joined late"] += 1
                    return  # not the battle's first request: the party may have changed order
                team = P.team_from_request(request, start_lines)
                wb = nb.WorldBuilder(side, team, set_pool, seed=1)
                wb.push_lines(start_lines)
            choice = sent.get(request.get("rqid"))
            if choice is None:
                stats["no choice sent"] += 1
                return
            if choice == "default":
                stats["default choices"] += 1  # the engine replays Showdown's autoChoose itself
            else:
                codes = P.codes_from_choice(request, choice)
                assert P.request_mask(request)[1][codes], (choice, codes)
                stats["choices"] += 1
            if decisions == 0 or (check_every and decisions % check_every == 0):
                wb.push_request(text)
                try:
                    wb.worlds(1, seed=decisions)
                    stats["worlds"] += 1
                except RuntimeError:
                    assert decisions > 0, wb.last_failure  # the first decision checks the reconstructed team
                    stats["no worlds"] += 1
            decisions += 1
            pending = (text, choice)
    stats["battles"] += 1


def test_recorded_server_streams():
    """Recorded server streams (scratch/showdown-logs, from the play harness): normalized lines carry only engine
    commands and normalized timestamps, and a WorldBuilder built from the team of the first request and fed only
    the normalized lines, requests and the bot's choices reproduces the requests."""
    files = recorded_battles()
    if not files:
        pytest.skip("no recorded server streams")
    stats = Counter()
    set_pool = nb.SetPool([str(POOL_200K)]) if nb.SEARCH else None
    for path in files:
        try:
            record = json.loads(path.read_text())
        except json.JSONDecodeError:
            continue  # still being written
        lines = [line for m in record["messages"] for line in m["text"].split("\n")]
        normal = P.normalize_lines(lines)
        commands = {line.split("|")[1] for line in normal}
        assert commands <= P.ENGINE_COMMANDS | {c for c in commands if c.startswith("-")}
        assert all(line == "|t:|" for line in normal if line.startswith("|t:|"))
        assert sum(line.startswith("|player|") for line in normal) == 2
        if set_pool is not None:
            # By default only the first decision is checked (it depends on the reconstructed team); every one with
            # NUMBRION_TEST_BATTLES set.
            replay_recording(record, set_pool, check_every=1 if os.environ.get("NUMBRION_TEST_BATTLES") else 0,
                             stats=stats)
    print(f"\nrecorded streams: {len(files)} files, {dict(stats)}")


def test_revival_blessing_slot_never_offers_pass():
    """A forced pass on a Revival Blessing slot is the Showdown bug behind "Choices are done immediately after a
    request"; the engine's masks leave it out (mask.rs passes_on_revival), and so must the request mask."""
    def mon(name, condition, active=False, reviving=False):
        entry = {"ident": f"p1: {name}", "details": f"{name}, L80", "condition": condition, "active": active,
                 "stats": {}, "moves": [], "baseAbility": "", "item": "", "pokeball": "pokeball", "ability": ""}
        if reviving:
            entry["reviving"] = True
        return entry
    request = {"forceSwitch": [True, False], "side": {"name": "Alice", "id": "p1", "pokemon": [
        mon("Pawmot", "100/100", active=True, reviving=True), mon("Muk", "120/200", active=True),
        mon("Abra", "0 fnt"), mon("Mew", "0 fnt"), mon("Ditto", "0 fnt"), mon("Eevee", "0 fnt")]}, "rqid": 3}
    mask0, joint = P.request_mask(request)
    assert not mask0[P.PASS]
    assert mask0[P.SWITCH_BASE + 2] and joint[P.SWITCH_BASE + 2, P.PASS]
