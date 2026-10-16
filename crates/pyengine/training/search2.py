"""Two-ply search: a second layer on top of the one-turn search (training/search.py), docs/design/SEARCH2-PLAN.md.

After layer 1 (``run_search``) we have our strategy x, each world's opponent strategy y_k and the payoffs A1. Layer 2:

1. Shortlist: our top ``shortlist`` pairs by x, plus the best pair by layer-1 q.
2. Sequential halving (``Depth2Config.rounds``): each round keeps the best survivors by the current estimate and
   deepens them on the first ``worlds`` of a fixed random order of the layer-1 worlds, with ``chance`` draws each
   (draw c < layer-1 seeds reuses layer 1's seed of that world). Samples are nested, so a round only adds new ones.
3. Replies per (world, candidate): the top ``theirs_top`` by y_k, plus ``danger`` replies with the lowest A1 among
   those with y_k >= ``danger_min``, plus (``threat_reply``) the most likely sleep-move reply (Spore, Yawn, ...) with
   y_k >= ``danger_min`` when none of those uses one: one ply cannot see what sleep costs, so A1 rarely marks it.
4. Expand (``expand``): every (world, candidate, reply, chance draw) cell is simulated to the next boundary, keeping
   the leaf battles, each side's leaf tracker (root tracker fed with that side's lines), requests, masks and policy
   inputs. Forced replacements are answered by the policy argmax and stepped through (``replacement_steps``), so
   layer 2 starts at a move turn. Ended leaves score +1/-1/0.
5. Layer 2 at the leaves: our candidates from our leaf policy (request masks), the opponent's from its leaf policy,
   a SearchGrid call from the leaves with ``seeds`` chance seeds per leaf, leaf values from the value head.
6. Our layer-2 strategy is shared by the leaves that look the same to us (``info="shared"``: same root pair, same
   observed opponent reply, same chance draw, across worlds): one Bayesian game per group, as at layer 1.
   ``"independent"`` solves each leaf alone. A leaf's value is x2 . q2 (``"mixed"``) or q2 at argmax x2.
7. Backup: ``"correction"`` Q2[i] = Q1[i] + mean_worlds sum_{j deepened} y_k[j] (V2[k,i,j] - A1[k,i,j]) (the
   opponent keeps its layer-1 weights); ``"resolve"`` re-solves the reduced game.
8. Pick: the best survivor of the last finished round, by the score halving also uses. Q2 is clipped to [-1, 1];
   ``pick="anchored"`` (default) scores log(prior[i]) + Q2[i] / anchor, layer 1's KL-anchored best response on the
   deeper values; ``pick="q"`` (or anchor 0) takes argmax Q2.

Work runs in bounded chunks (a grid call, a network batch, a solve); ``StageClock`` refuses a chunk whose predicted
cost would pass the deadline, and the result of the last finished round is used (none: keep layer 1).
"""
from __future__ import annotations

import contextlib
import json
import time
from dataclasses import dataclass, field

import numpy as np

from .protocol import request_mask
from .search2_types import Depth2Config, Expansion, HalvingRound, Layer1, Stage  # noqa: F401 - re-exported

N_ACTIONS = 47
PASS = 46
NONE_PAIR = (-1, -1)
POLICY_KEYS = ("ids", "floats", "field", "token_features", "action_features", "guard")
VALUE_KEYS = ("ids", "floats", "field", "token_features")
# Prior per-unit costs (ms) for deadline checks before the first measurement. Units: expand/grid2 cells, leaf_policy
# and value2 rows, solve2 groups.
DEFAULT_UNIT_MS = {"expand": 0.1, "leaf_policy": 0.6, "grid2": 0.3, "value2": 0.1, "solve2": 0.5}
CHUNK_FIXED_MS = 15.0     # predicted overhead of any chunk
CHUNK_SAFETY = 2.0        # conservative bound: predicted = SAFETY * units * unit_ms + FIXED
DEADLINE_MARGIN_MS = 120.0  # kept back from the deadline for the pick, the analysis and the send
GRID2_CHUNK_CELLS = 4096  # cells per layer-2 grid call (one uninterruptible unit)


# ---- time ----------------------------------------------------------------------------------------------------------

class StageClock:
    """Times layer-2 stages (``stages``: list of ``Stage``) and says whether the next chunk may start.

    With a budget controller, ``next_chunk_ok`` decides. Without one and without a deadline, every chunk runs (fixed
    sizes). Without a controller but with a deadline, a running per-unit cost (``costs``, ms per unit by stage, kept
    by the caller across decisions) predicts the chunk conservatively."""

    def __init__(self, budget=None, deadline: float | None = None, costs: dict | None = None):
        self.budget, self.deadline = budget, deadline
        self.costs = costs if costs is not None else {}
        self.stages: list[Stage] = []
        self.refused: str | None = None

    def predict_ms(self, stage: str, units: int) -> float:
        return CHUNK_SAFETY * units * self.costs.get(stage, DEFAULT_UNIT_MS.get(stage, 0.5)) + CHUNK_FIXED_MS

    def ok(self, stage: str, units: int) -> bool:
        if units <= 0:
            return True
        now = time.monotonic()
        if self.budget is not None:
            ok = bool(self.budget.next_chunk_ok(stage, int(units), now, self.deadline))
        elif self.deadline is None:
            ok = True
        else:
            ok = now + (self.predict_ms(stage, units) + DEADLINE_MARGIN_MS) / 1000.0 <= self.deadline
        if not ok:
            self.refused = f"{stage} ({int(units)} units) refused by the {'budget' if self.budget else 'deadline'}"
        return ok

    def admit(self, names: tuple, units: int) -> int:
        """The largest chunk (``units``, halved until admitted) that every stage of ``names`` may start now; 0 if
        not even one unit may."""
        n = int(units)
        while n >= 1:
            if all(self.ok(name, n) for name in names):
                self.refused = None
                return n
            n //= 2
        return 0

    def record(self, name: str, units: int, ms: float) -> None:
        self.stages.append(Stage(name, int(units), ms))
        if units > 0:
            per = ms / units
            self.costs[name] = per if name not in self.costs else self.costs[name] + 0.2 * (per - self.costs[name])

    @contextlib.contextmanager
    def stage(self, name: str, units: int):
        t = time.perf_counter()
        yield
        self.record(name, units, (time.perf_counter() - t) * 1000.0)

    def totals(self) -> dict:
        out: dict = {}
        for s in self.stages:
            e = out.setdefault(s.name, [0, 0.0])
            e[0] += s.units
            e[1] += s.ms
        return {k: dict(units=u, ms=round(ms, 1)) for k, (u, ms) in out.items()}


# ---- expansion -----------------------------------------------------------------------------------------------------

def _tracker_pairs(states, n_roots: int) -> list:
    if len(states) == 2 and not isinstance(states[0], (tuple, list)):
        return [tuple(states)] * n_roots
    if len(states) != n_roots:
        raise ValueError(f"states: one pair or one per root ({n_roots}), got {len(states)}")
    return [tuple(s) for s in states]


def expand(bot, worlds, actions, world_idx=None, seeds=None, states=None, side: int = 0) -> Expansion:
    """Step cells from their roots to the next decision boundary and keep the leaves for a further search
    (``search2_types.Expansion``). ``worlds``: root battles (log=True); ``actions`` [n, 2, 2]; ``world_idx`` [n]
    (default 0); ``seeds`` None (each root's PRNG state), [4] or [n, 4]; ``states``: the root trackers, one
    (side 0, side 1) pair for all roots or one per root. ``side`` is the searching side (it does not change the
    result; kept for the interface). Uses the fused ``SearchGrid.expand`` when the extension has it
    (``numbrion.FUSED_EXPAND``), else this Python reference: GridExecutor + TrackerState.feed + encode_states.
    ``requests`` are the engine's request JSON of both sides as GridExecutor returns them; the policy rows and
    ``request_joint`` of a side that needs no action are zero."""
    import numbrion

    del side
    local = bot.resources()
    actions = np.asarray(actions, dtype=np.int32).reshape(-1, 2, 2)
    n = len(actions)
    world_idx = np.zeros(n, dtype=np.int64) if world_idx is None else np.asarray(world_idx, dtype=np.int64)
    seeds = None if seeds is None else np.asarray(seeds, dtype=np.int64)
    pairs = _tracker_pairs(states, len(worlds))
    timing = {}
    t0 = time.perf_counter()
    if getattr(numbrion, "FUSED_EXPAND", False) and hasattr(local.grid, "expand"):
        out = local.grid.expand(list(worlds), actions, [list(p) for p in pairs], world=world_idx, seeds=seeds)
        timing["fused_ms"] = round((time.perf_counter() - t0) * 1000, 2)
        fields = {k: out[k] for k in Expansion.__dataclass_fields__ if k in out and k != "timing"}
        fields["trackers"] = [None if t is None else tuple(t) for t in fields["trackers"]]
        fields["requests"] = [tuple(r) for r in fields["requests"]]
        return Expansion(**fields, timing=timing)
    kwargs = dict(world=world_idx, keep_leaves=True, logs=True, requests=True)
    if seeds is not None:
        kwargs["seeds"] = seeds
    out = local.executor.run(list(worlds), actions, **kwargs)
    timing["grid_ms"] = (time.perf_counter() - t0) * 1000
    t1 = time.perf_counter()
    ended = np.asarray(out["ended"], dtype=bool)
    winner = np.asarray(out["winner"], dtype=np.int8)
    needs = np.asarray(out["needs_action"], dtype=bool).reshape(n, 2)
    kind = np.asarray(out["request_kind"], dtype=np.uint8).reshape(n, 2)
    leaves, trackers = [], []
    requests = [tuple(r) for r in out["requests"]]
    request_joint = np.zeros((n, 2, N_ACTIONS, N_ACTIONS), dtype=bool)
    engine_joint = np.zeros((n, 2, N_ACTIONS, N_ACTIONS), dtype=bool)
    rows = []
    for c in range(n):
        if ended[c]:
            leaves.append(None)
            trackers.append(None)
            continue
        root = pairs[int(world_idx[c])]
        logs = out["logs"][c]
        trackers.append((root[0].feed(list(logs[0])), root[1].feed(list(logs[1]))))
        leaf = out["leaves"][c]
        leaves.append(leaf)
        for s in (0, 1):
            engine_joint[c, s] = leaf.legal_mask(s)[1]
            if needs[c, s]:
                request_joint[c, s] = request_mask(requests[c][s])[1]
                rows.append((c, s))
    timing["feed_ms"] = (time.perf_counter() - t1) * 1000
    t2 = time.perf_counter()
    policy = {}
    if rows:
        enc = local.grid.encode_states([trackers[c][s] for c, s in rows], [requests[c][s] for c, s in rows])
        idx_c = np.array([c for c, _ in rows])
        idx_s = np.array([s for _, s in rows])
        for key in POLICY_KEYS:
            arr = np.zeros((n, 2) + enc[key].shape[1:], dtype=enc[key].dtype)
            arr[idx_c, idx_s] = enc[key]
            policy[key] = arr
    timing["encode_ms"] = (time.perf_counter() - t2) * 1000
    return Expansion(ended=ended, winner=winner, needs_action=needs, request_kind=kind, leaves=leaves,
                     trackers=trackers, requests=requests, policy=policy, request_joint=request_joint,
                     engine_joint=engine_joint, timing={k: round(v, 2) for k, v in timing.items()})


# ---- batched Bayesian solver ----------------------------------------------------------------------------------------

def _softmax_last(z: np.ndarray) -> np.ndarray:
    m = np.max(z, axis=-1, keepdims=True)
    m = np.where(np.isfinite(m), m, 0.0)
    e = np.exp(z - m)
    return e / np.maximum(e.sum(-1, keepdims=True), 1e-300)


def solve_groups(payoffs: np.ndarray, valid: np.ndarray, world_mask: np.ndarray, our_prior: np.ndarray,
                 their_prior: np.ndarray, anchor: float, opponent_anchor: float, iterations: int):
    """``search.solve`` for G padded Bayesian games at once. ``payoffs`` [G, W, N, M]; ``valid`` [G, W, N];
    ``world_mask`` [G, W] (False: padding world); ``our_prior`` [G, N] (0: padding pair); ``their_prior`` [G, W, M]
    probabilities (0: padding reply). Returns (x [G, N], ys [G, W, M], q [G, N]) as ``solve`` does per game."""
    G, W, N, M = payoffs.shape
    ok = valid & world_mask[:, :, None]
    allowed = ok.any(1) & (our_prior > 0)
    p = np.where(allowed, our_prior, 0.0)
    p = p / np.maximum(p.sum(1, keepdims=True), 1e-12)
    with np.errstate(divide="ignore"):
        log_x0 = np.where(allowed, np.log(np.maximum(p, 1e-300)), -np.inf)
    theirs_ok = their_prior > 0
    tp = np.where(world_mask[:, :, None], their_prior, 0.0)
    tp[~world_mask] = 0.0
    tp[~world_mask, 0] = 1.0
    tp = tp / np.maximum(tp.sum(-1, keepdims=True), 1e-300)
    theirs_ok = theirs_ok | (~world_mask[:, :, None] & (np.arange(M) == 0))
    with np.errstate(divide="ignore"):
        log_y0 = np.where(theirs_ok, np.log(np.maximum(tp, 1e-12)), -np.inf)
    a = np.where(theirs_ok[:, :, None, :], payoffs, 0.0)
    weight = ok.sum(1)

    def our_values(y):
        v = np.einsum("gwnm,gwm->gwn", a, y)
        total = np.where(ok, v, 0.0).sum(1)
        return np.where(weight > 0, total / np.maximum(weight, 1e-12), -2.0)

    def respond(q):
        if anchor > 0:
            return _softmax_last(np.where(allowed, log_x0 + q / anchor, -np.inf))
        qm = np.where(allowed, q, -np.inf)
        best = (qm == qm.max(1, keepdims=True)) & allowed
        return best / np.maximum(best.sum(1, keepdims=True), 1)

    ys = tp.copy()
    if opponent_anchor <= 0:
        q = our_values(ys)
        return respond(q), ys, q
    x_avg = p.copy()
    y_avg = ys
    for t in range(1, iterations + 1):
        x = respond(our_values(y_avg))
        x_avg += (x - x_avg) / (t + 1)
        xs = np.where(ok, x_avg[:, None, :], 0.0)
        xs = xs / np.maximum(xs.sum(-1, keepdims=True), 1e-12)
        y = _softmax_last(log_y0 - np.einsum("gwn,gwnm->gwm", xs, a) / opponent_anchor)
        y_avg += (y - y_avg) / (t + 1)
    return x_avg, y_avg, our_values(y_avg)


# ---- leaves --------------------------------------------------------------------------------------------------------

@dataclass
class _Leaf:
    key: tuple                      # (world k, root candidate i, reply j, chance draw c)
    ended: bool = False
    terminal: float | None = None   # +1/-1/0 for an ended leaf
    battle: object = None
    trackers: tuple | None = None
    requests: tuple = (None, None)
    needs: tuple = (False, False)
    kind: tuple = (0, 0)
    request_joint: np.ndarray | None = None   # [2, 47, 47]
    engine_joint: np.ndarray | None = None
    exp: Expansion | None = None    # policy inputs: exp.policy[key][row]
    row: int = 0
    steps: int = 0                  # forced replacements stepped through
    ours: list = field(default_factory=list)   # our leaf candidates (independent mode)
    full: dict = field(default_factory=dict)    # our leaf policy: (a0, a1) -> (logp T=1, logp at T)
    their: np.ndarray | None = None  # [M, 2] opponent pairs ((-1, -1) if it needs no action)
    their_prior: np.ndarray | None = None
    seeds2: np.ndarray | None = None
    rows: dict = field(default_factory=dict)    # our pair -> [M] values (mean over seeds2)
    value: float | None = None      # V2
    failed: str = ""


def _set_from(leaf: _Leaf, exp: Expansion, c: int, side: int) -> None:
    if exp.ended[c]:
        w = int(exp.winner[c])
        leaf.ended, leaf.terminal = True, 1.0 if w == side else (0.0 if w < 0 else -1.0)
        leaf.battle = leaf.trackers = leaf.exp = None
        leaf.value = leaf.terminal
        return
    leaf.battle, leaf.trackers, leaf.requests = exp.leaves[c], exp.trackers[c], exp.requests[c]
    leaf.needs = (bool(exp.needs_action[c, 0]), bool(exp.needs_action[c, 1]))
    leaf.kind = (int(exp.request_kind[c, 0]), int(exp.request_kind[c, 1]))
    leaf.request_joint, leaf.engine_joint = exp.request_joint[c], exp.engine_joint[c]
    leaf.exp, leaf.row = exp, c


def _forced(leaf: _Leaf) -> bool:
    return not leaf.ended and 1 not in leaf.kind and 2 in leaf.kind


def _policy_arrays(leaves: list, s: int) -> dict:
    out = {}
    for key in POLICY_KEYS:
        out[key] = np.stack([lf.exp.policy[key][lf.row, s] for lf in leaves])
    return out


def _side_mask(leaf: _Leaf, s: int, our_side: int) -> np.ndarray:
    """Policy mask of side ``s`` at the leaf: ours from the request; the opponent's from its request and the
    engine (a refused opponent pair could not be simulated); either falls back to the engine if empty."""
    req, eng = leaf.request_joint[s], leaf.engine_joint[s]
    if s == our_side:
        return req if req.any() else eng
    both = req & eng
    return both if both.any() else eng


def _candidates(bot, leaves: list, s: int, our_side: int, k: int, first: int, temperature: float):
    if not leaves:
        return []
    arrays = _policy_arrays(leaves, s)
    masks = []
    for lf in leaves:
        joint = _side_mask(lf, s, our_side)
        masks.append((joint.any(1), joint))
    return bot.net.candidates(arrays, np.arange(len(leaves)), masks, k=k, first=first, temperature=temperature)


LOG_FLOOR = float(np.log(1e-6))  # log-prior of a pair a sibling's policy does not allow, for ranking


def common_opponent_sets(tops: list, fulls: list, masks: list, size: int) -> list:
    """One opponent candidate set for sibling leaves (the same world, opponent reply and chance draw, after different
    root pairs of ours), so our candidates are compared against the same replies.

    ``tops[l]``: leaf l's own top pairs (most likely first); ``fulls[l]``: its policy, (a0, a1) -> (log-prob at T=1,
    log-prob at the opponent temperature); ``masks[l]`` [47, 47]: the pairs its opponent may play. The set is the union
    of the leaves' top pairs ranked by their log-prior averaged over the leaves (LOG_FLOOR where a leaf does not
    allow a pair), cut to ``size``, plus at most ``size`` more for coverage: every leaf keeps its own most likely pair
    and at least min(2, its top count) pairs it allows. Returns per leaf (pairs [m, 2] int32, prior [m]): the common
    pairs that leaf allows, with its own prior renormalised over them."""
    union = []
    for top in tops:
        for pair in top:
            if pair not in union:
                union.append(pair)

    def mean_logp(pair):
        return float(np.mean([full[pair][0] if pair in full else LOG_FLOOR for full in fulls]))

    common = sorted(union, key=lambda pair: (-mean_logp(pair), pair))[:max(size, 1)]
    extra = 0
    for top, full, mask in zip(tops, fulls, masks):
        def covered():
            usable = sum(1 for pair in common if pair in full and mask[pair])
            return (not top or top[0] in common) and usable >= min(2, len(top))
        for pair in top:
            if covered() or extra >= size:
                break
            if pair not in common:
                common.append(pair)
                extra += 1
    out = []
    for top, full, mask in zip(tops, fulls, masks):
        pairs = [pair for pair in common if pair in full and mask[pair]] or list(top[:1])
        logp = np.array([full[pair][1] for pair in pairs], dtype=float)
        prior = np.exp(logp - logp.max())
        out.append((np.asarray(pairs, dtype=np.int32).reshape(-1, 2), prior / prior.sum()))
    return out


# ---- the second layer ----------------------------------------------------------------------------------------------

class _Deepen:
    def __init__(self, bot, cfg, cfg2: Depth2Config, layer1: Layer1, clock: StageClock, rng, pick: str,
                 ours_shown: tuple, describe, threat_reply: bool = False):
        self.bot, self.cfg, self.cfg2, self.l1, self.clock, self.rng = bot, cfg, cfg2, layer1, clock, rng
        self.pick, self.ours_shown, self.describe = pick, ours_shown, describe
        self.threat_reply = threat_reply
        self.common_opponent = bool(getattr(cfg, "depth2_common_opponent", True))
        self.log_prior = np.log(np.maximum(np.asarray(layer1.our_prior, dtype=float), 1e-300))
        self.requests: dict = {}
        self.side = layer1.side
        self.opp = 1 - layer1.side
        n_worlds = len(layer1.worlds)
        self.K = n_worlds
        legal = np.asarray(layer1.legal, dtype=bool).reshape(n_worlds, -1)
        self.legal = legal
        somewhere = legal.any(0)
        self.imputed_to = {}
        for k in range(n_worlds):
            if legal[k].any():
                rows = np.flatnonzero(legal[k])
                prior = layer1.their_prior[k] if layer1.their_prior[k] is not None else np.ones(1)
                best = int(rows[np.argmax(layer1.payoffs[k][rows] @ prior)])
                for i in np.flatnonzero(somewhere & ~legal[k]):
                    self.imputed_to[(k, int(i))] = best
        self.perm = rng.permutation(n_worlds)
        self.seeds: dict = {}
        self.leaves: dict = {}
        self.replies: dict = {}
        self.obs: dict = {}
        self.cells = dict(expand=0, grid2=0)
        self.forced_steps = 0

    # -- samples
    def seed(self, k: int, c: int) -> np.ndarray:
        if (k, c) not in self.seeds:
            s1 = self.l1.seeds[k]
            self.seeds[(k, c)] = (np.asarray(s1[c], dtype=np.int64) if c < len(s1)
                                  else self.rng.integers(0, 1 << 16, size=4).astype(np.int64))
        return self.seeds[(k, c)]

    def reply_set(self, k: int, i: int) -> list:
        """(j, danger) of the replies deepened in world k for our candidate i."""
        if (k, i) not in self.replies:
            y = np.asarray(self.l1.ys[k], dtype=float)
            if self.l1.their[k] is None:
                out = [(0, False)]
            else:
                order = np.argsort(-y, kind="stable")
                top = [int(j) for j in order[:self.cfg2.theirs_top]]
                rest = [j for j in range(len(y)) if j not in top and y[j] >= self.cfg2.danger_min]
                a1 = self.l1.payoffs[k][i]
                danger = sorted(rest, key=lambda j: (a1[j], j))[:max(self.cfg2.danger, 0)]
                out = [(j, False) for j in top] + [(int(j), True) for j in danger]
                if self.threat_reply:
                    sleepy = [j for j in range(len(y)) if y[j] >= self.cfg2.danger_min and self.sleep_reply(k, j)]
                    if sleepy and not any(j in sleepy for j, _ in out):
                        out.append((max(sleepy, key=lambda j: (y[j], -j)), True))
            self.replies[(k, i)] = out
        return self.replies[(k, i)]

    def opp_request(self, k: int) -> dict:
        if k not in self.requests:
            self.requests[k] = json.loads(self.l1.worlds[k].request_json(self.opp))
        return self.requests[k]

    def sleep_reply(self, k: int, j: int) -> bool:
        from .search import SLEEP_MOVES, reply_moves

        return bool(set(reply_moves(self.opp_request(k), self.l1.their[k][j])) & SLEEP_MOVES)

    def obs_text(self, k: int, j: int) -> str:
        """What we would see of reply j in world k (move names, targets, switch-ins): the info-group key."""
        if (k, j) not in self.obs:
            from .search import describe_code

            if self.l1.their[k] is None:
                text = "none"
            else:
                request = self.opp_request(k)
                names = [p["details"].split(",")[0] for p in request.get("side", {}).get("pokemon", [])[:2]]
                codes = [int(c) for c in self.l1.their[k][j]]
                parts = [f"{names[s]}: {describe_code(request, s, c, self.ours_shown)}" for s, c in enumerate(codes)
                         if c != PASS and c >= 0 and s < len(names)]
                text = "; ".join(parts) or "pass"
            self.obs[(k, j)] = text
        return self.obs[(k, j)]

    # -- stages
    def expand_chunks(self, roots, actions, world_idx, seeds, states) -> list | None:
        """``expand`` in admitted chunks: [(Expansion, first cell)], or None when a chunk was refused."""
        out, begin, n = [], 0, len(actions)
        while begin < n:
            size = self.clock.admit(("expand",), n - begin)
            if size == 0:
                return None
            part = slice(begin, begin + size)
            with self.clock.stage("expand", size):
                exp = expand(self.bot, roots, actions[part], world_idx[part],
                             None if seeds is None else seeds[part], states, self.side)
            out.append((exp, begin))
            self.cells["expand"] += size
            begin += size
        return out

    def policy_chunks(self, leaves: list, s: int, k: int, temperature: float) -> list | None:
        """``_candidates`` for side ``s`` of ``leaves`` in admitted network batches; None when one was refused."""
        found, begin = [], 0
        while begin < len(leaves):
            size = self.clock.admit(("leaf_policy",), len(leaves) - begin)
            if size == 0:
                return None
            with self.clock.stage("leaf_policy", size):
                found += _candidates(self.bot, leaves[begin:begin + size], s, self.side, k, self.cfg.first_codes,
                                     temperature)
            begin += size
        return found

    def grow(self, keys: list) -> bool:
        """Expand the new leaves ``keys``, step forced replacements, run both sides' leaf policies."""
        l1 = self.l1
        if keys:
            actions = np.full((len(keys), 2, 2), -1, dtype=np.int32)
            for n, (k, i, j, c) in enumerate(keys):
                actions[n, self.side] = l1.ours[i]
                if l1.their[k] is not None:
                    actions[n, self.opp] = l1.their[k][j]
            parts = self.expand_chunks(l1.worlds, actions, np.array([k for k, _, _, _ in keys]),
                                       np.stack([self.seed(k, c) for k, _, _, c in keys]), l1.states)
            if parts is None:
                return False
            for exp, begin in parts:
                for c in range(len(exp.ended)):
                    leaf = _Leaf(key=keys[begin + c])
                    _set_from(leaf, exp, c, self.side)
                    self.leaves[leaf.key] = leaf
        new = [self.leaves[k] for k in keys]
        for _ in range(max(self.cfg2.replacement_steps, 0)):
            forced = [lf for lf in new if _forced(lf)]
            if not forced:
                break
            if not self.step(forced):
                return False
        live = [lf for lf in new if not lf.ended]
        cfg, cfg2 = self.cfg, self.cfg2
        ours_rows = [lf for lf in live if lf.needs[self.side]]
        found = self.policy_chunks(ours_rows, self.side, cfg2.ours, self.bot.policy_temperature)
        if found is None:
            return False
        for lf, (pairs, _, full) in zip(ours_rows, found):
            lf.ours = [tuple(int(v) for v in p) for p in pairs]
            lf.full = full
        theirs_rows = [lf for lf in live if lf.needs[self.opp]]
        found = self.policy_chunks(theirs_rows, self.opp, cfg2.theirs, cfg.opponent_temperature)
        if found is None:
            return False
        if self.common_opponent:  # siblings (same world, reply and chance draw) share one opponent candidate set
            siblings: dict = {}
            for lf, cands in zip(theirs_rows, found):
                k, _, j, c = lf.key
                siblings.setdefault((k, j, c), []).append((lf, cands))
            for members in siblings.values():
                sets = common_opponent_sets([[tuple(int(v) for v in p) for p in pairs] for _, (pairs, _, _) in members],
                                            [full for _, (_, _, full) in members],
                                            [_side_mask(lf, self.opp, self.side) for lf, _ in members], cfg2.theirs)
                for (lf, _), (pairs, prior) in zip(members, sets):
                    lf.their, lf.their_prior = pairs, prior
        else:
            for lf, (pairs, logp, _) in zip(theirs_rows, found):
                prior = np.exp(logp - logp.max())
                lf.their, lf.their_prior = np.asarray(pairs, dtype=np.int32), prior / prior.sum()
        for lf in live:
            if not lf.needs[self.opp]:
                lf.their, lf.their_prior = np.full((1, 2), -1, dtype=np.int32), np.ones(1)
            if lf.needs[self.side] and not lf.ours:
                lf.failed = "no legal pair for us at the leaf"
            lf.seeds2 = self.rng.integers(0, 1 << 16, size=(max(self.cfg2.seeds, 1), 4)).astype(np.int64)
        return True

    def step(self, forced: list) -> bool:
        """Answer the forced replacements of ``forced`` leaves with each side's policy argmax and step on."""
        actions = np.full((len(forced), 2, 2), -1, dtype=np.int32)
        for s in (0, 1):
            rows = [n for n, lf in enumerate(forced) if lf.needs[s]]
            found = self.policy_chunks([forced[n] for n in rows], s, 1, 1.0)
            if found is None:
                return False
            for n, (pairs, _, _) in zip(rows, found):
                engine = forced[n].engine_joint[s]
                pair = pairs[0] if len(pairs) else None
                if pair is None or not engine[pair[0], pair[1]]:  # nothing allowed by the policy mask: engine's first
                    pair = np.argwhere(engine)[0]
                actions[n, s] = pair
        parts = self.expand_chunks([lf.battle for lf in forced], actions, np.arange(len(forced)), None,
                                   [lf.trackers for lf in forced])
        if parts is None:
            return False
        self.forced_steps += len(forced)
        for exp, begin in parts:
            for c in range(len(exp.ended)):
                lf = forced[begin + c]
                _set_from(lf, exp, c, self.side)
                lf.steps += 1
        return True

    def groups(self, keys: list) -> dict:
        out: dict = {}
        for key in keys:
            leaf = self.leaves[key]
            if leaf.ended or leaf.failed:
                continue
            k, i, j, c = key
            gkey = (i, self.obs_text(k, j), c) if self.cfg2.info == "shared" else key
            out.setdefault(gkey, []).append(leaf)
        return out

    def group_pairs(self, leaves: list) -> tuple[list, np.ndarray]:
        """Our candidate pairs for a group and their prior: each acting leaf's most likely pair, then the most
        likely pairs on average over the group's acting leaves (``ours`` in all, at least one per leaf)."""
        acting = [lf for lf in leaves if lf.needs[self.side]]
        if not acting:
            return [NONE_PAIR], np.ones(1)
        if self.cfg2.info != "shared":
            pairs = list(acting[0].ours)
        else:
            mean: dict = {}
            for lf in acting:
                for pair, (logp, _) in lf.full.items():
                    mean[pair] = mean.get(pair, 0.0) + float(np.exp(logp)) / len(acting)
            ranked = sorted(mean, key=lambda p: (-mean[p], p))
            pairs = []
            for lf in acting:
                top = lf.ours[0]
                if top not in pairs:
                    pairs.append(top)
            pairs.sort(key=lambda p: (-mean.get(p, 0.0), p))
            size = max(self.cfg2.ours, len(pairs))
            for p in ranked:
                if len(pairs) >= size:
                    break
                if p not in pairs:
                    pairs.append(p)
        prob = np.zeros(len(pairs))
        for lf in acting:
            for n, p in enumerate(pairs):
                if p in lf.full:
                    prob[n] += float(np.exp(lf.full[p][0])) / len(acting)
        prob = np.maximum(prob, 1e-12) ** (1.0 / max(self.cfg.prior_temperature, 1e-6))
        return pairs, prob / prob.sum()

    def layer2(self, keys: list) -> bool:
        """Cells, values and solves for the leaves ``keys`` (all leaves of the round); sets each leaf's value."""
        groups = self.groups(keys)
        plan = []  # (group key, leaves, pairs, prior)
        needed = []  # (leaf, our pair)
        for gkey, leaves in groups.items():
            pairs, prior = self.group_pairs(leaves)
            plan.append((gkey, leaves, pairs, prior))
            for lf in leaves:
                if not lf.needs[self.side]:
                    if NONE_PAIR not in lf.rows:
                        needed.append((lf, NONE_PAIR))
                    continue
                req, eng = lf.request_joint[self.side], lf.engine_joint[self.side]
                for p in pairs:
                    if p not in lf.rows and req[p] and eng[p]:
                        needed.append((lf, p))
        if not self.run_cells(needed):
            return False
        if not self.clock.ok("solve2", len(plan)):  # one uninterruptible solve of every group
            return False
        with self.clock.stage("solve2", len(plan)):
            self.solve(plan)
        return True

    def run_cells(self, needed: list) -> bool:
        """Layer-2 cells for the (leaf, our pair) rows ``needed``: every opponent candidate x ``seeds`` chance seeds
        of the leaf, in admitted grid calls (at most GRID2_CHUNK_CELLS) and value batches."""
        from .search import grid_values

        if not needed:
            return True
        seeds2 = max(self.cfg2.seeds, 1)
        cells = []  # (n of needed, leaf, j, s)
        for n, (lf, _) in enumerate(needed):
            for j in range(len(lf.their)):
                for s in range(seeds2):
                    cells.append((n, lf, j, s))
        sums = [np.zeros(len(lf.their)) for lf, _ in needed]
        grid = self.bot.resources().grid
        begin = 0
        while begin < len(cells):
            size = self.clock.admit(("grid2", "value2"), min(GRID2_CHUNK_CELLS, len(cells) - begin))
            if size == 0:
                return False
            chunk = cells[begin:begin + size]
            roots, index = [], {}
            for _, lf, _, _ in chunk:
                if id(lf) not in index:
                    index[id(lf)] = len(roots)
                    roots.append(lf)
            actions = np.full((len(chunk), 2, 2), -1, dtype=np.int32)
            world = np.empty(len(chunk), dtype=np.int64)
            seeds = np.empty((len(chunk), 4), dtype=np.int64)
            for m, (n, lf, j, s) in enumerate(chunk):
                pair = needed[n][1]
                if pair != NONE_PAIR:
                    actions[m, self.side] = pair
                actions[m, self.opp] = lf.their[j]
                world[m] = index[id(lf)]
                seeds[m] = lf.seeds2[s]
            with self.clock.stage("grid2", len(chunk)):
                out = grid.run([lf.battle for lf in roots], actions, [lf.trackers for lf in roots], self.side,
                               world=world, seeds=seeds)
            values = grid_values(self.bot.net, out, self.side, gate=self.clock.ok, record=self.clock.record,
                                 stage="value2")
            if values is None:
                return False
            self.cells["grid2"] += len(chunk)
            for (n, lf, j, s), v in zip(chunk, values):
                sums[n][j] += v
            begin += size
        for n, (lf, pair) in enumerate(needed):
            lf.rows[pair] = sums[n] / seeds2
        return True

    def leaf_game(self, lf: _Leaf, pairs: list):
        """(payoffs [N, M], valid [N]) of a leaf for our group pairs, refusals imputed as at layer 1; None if no
        pair can be simulated there."""
        N, M = len(pairs), len(lf.their)
        if not lf.needs[self.side]:
            return np.tile(lf.rows[NONE_PAIR], (N, 1)), np.ones(N, dtype=bool)
        req, eng = lf.request_joint[self.side], lf.engine_joint[self.side]
        a = np.zeros((N, M))
        valid = np.zeros(N, dtype=bool)
        refused = np.zeros(N, dtype=bool)
        for n, p in enumerate(pairs):
            if req[p] and eng[p]:
                a[n], valid[n] = lf.rows[p], True
            elif req[p]:
                refused[n] = True
        if not valid.any():
            return None
        if refused.any():
            rows = np.flatnonzero(valid)
            best = rows[np.argmax(a[rows] @ lf.their_prior)]
            a[refused] = a[best]
            valid |= refused
        return a, valid

    def solve(self, plan: list) -> None:
        cfg = self.cfg
        games = []
        for gkey, leaves, pairs, prior in plan:
            members = []
            for lf in leaves:
                game = self.leaf_game(lf, pairs)
                if game is None:  # none of the group's pairs can be simulated here: no layer-2 value (this round)
                    lf.value = None
                    continue
                members.append((lf, *game))
            if members:
                games.append((members, prior))
        if not games:
            return
        G = len(games)
        W = max(len(m) for m, _ in games)
        N = max(len(p) for _, p in games)
        M = max(len(lf.their) for m, _ in games for lf, _, _ in m)
        A = np.zeros((G, W, N, M))
        valid = np.zeros((G, W, N), dtype=bool)
        wmask = np.zeros((G, W), dtype=bool)
        ours = np.zeros((G, N))
        theirs = np.zeros((G, W, M))
        for g, (members, prior) in enumerate(games):
            ours[g, :len(prior)] = prior
            for w, (lf, a, ok) in enumerate(members):
                A[g, w, :a.shape[0], :a.shape[1]] = a
                valid[g, w, :len(ok)] = ok
                wmask[g, w] = True
                theirs[g, w, :len(lf.their_prior)] = lf.their_prior
        if self.cfg2.info == "shared":
            x, ys, _ = solve_groups(A, valid, wmask, ours, theirs, cfg.anchor, cfg.opponent_anchor, cfg.iterations)
            qs = None
        else:  # one leaf per game: solve_batch, with pairs the leaf does not allow left out of our prior
            from .search import solve_batch

            x, qs = solve_batch(A[:, 0], np.where(valid[:, 0], ours, 0.0), theirs[:, 0], cfg.anchor,
                                cfg.opponent_anchor, cfg.iterations)
            ys = None
        for g, (members, _) in enumerate(games):
            for w, (lf, a, ok) in enumerate(members):
                n, m = a.shape
                q = a @ ys[g, w, :m] if ys is not None else qs[g, :n]
                xg = np.where(ok, x[g, :n], 0.0)
                if self.cfg2.leaf_value == "argmax":
                    lf.value = float(q[int(np.argmax(np.where(ok, x[g, :n], -1.0)))])
                else:
                    lf.value = float(xg @ np.where(ok, q, 0.0) / max(xg.sum(), 1e-12))

    # -- backup
    def source(self, k: int, i: int, survivors: list) -> int:
        if self.legal[k][i]:
            return i
        b = self.imputed_to.get((k, i))
        return b if b is not None and b in survivors else i

    def v2(self, k: int, i: int, j: int, chance: int):
        vals = [self.leaves[(k, i, j, c)].value for c in range(chance) if (k, i, j, c) in self.leaves]
        vals = [v for v in vals if v is not None]
        return float(np.mean(vals)) if vals else None

    def backup(self, survivors: list, worlds: list, chance: int) -> tuple[dict, dict]:
        """(Q2 by candidate, pick score by candidate)."""
        l1, cfg = self.l1, self.cfg
        q1 = np.asarray(l1.q, dtype=float)
        if self.cfg2.backup == "resolve":
            payoffs, valid, their_priors = [], [], []
            for k in worlds:
                srcs = [self.source(k, i, survivors) for i in survivors]
                cols = sorted({j for i, s in zip(survivors, srcs) for j, _ in self.reply_set(k, s)})
                a = np.zeros((len(survivors), len(cols)))
                ok = np.zeros(len(survivors), dtype=bool)
                for r, (i, s) in enumerate(zip(survivors, srcs)):
                    ok[r] = self.legal[k][i] or (k, i) in self.imputed_to
                    for col, j in enumerate(cols):
                        v = self.v2(k, s, j, chance)
                        a[r, col] = v if v is not None else l1.payoffs[k][i][j]
                prior = l1.their_prior[k] if l1.their_prior[k] is not None else np.ones(1)
                p = np.asarray(prior, dtype=float)[cols]
                payoffs.append(a)
                valid.append(ok)
                their_priors.append(p / max(p.sum(), 1e-12))
            from .search import solve

            our_prior = np.asarray(l1.our_prior, dtype=float)[survivors]
            _, _, q2 = solve(payoffs, valid, np.ones(len(worlds)), our_prior / our_prior.sum(), their_priors,
                             cfg.anchor, cfg.opponent_anchor, cfg.iterations)
            Q2 = {i: float(np.clip(q2[r], -1.0, 1.0)) for r, i in enumerate(survivors)}
            return Q2, {i: self.score(i, Q2[i]) for i in survivors}
        Q2, score = {}, {}
        for i in survivors:
            terms = []
            for k in worlds:
                s = self.source(k, i, survivors)
                y = np.asarray(l1.ys[k], dtype=float)
                term, any_ = 0.0, False
                for j, _ in self.reply_set(k, s):
                    v = self.v2(k, s, j, chance)
                    if v is not None:
                        term += y[j] * (v - float(l1.payoffs[k][s][j]))
                        any_ = True
                if any_:
                    terms.append(term)
            delta = float(np.mean(terms)) if terms else 0.0
            # The correction is averaged over the deepened worlds only, so Q1 + delta can leave [-1, 1].
            Q2[i] = float(np.clip(q1[i] + delta, -1.0, 1.0))
            score[i] = self.score(i, Q2[i])
        return Q2, score

    def score(self, i: int, value: float) -> float:
        """The pick and halving score of candidate i with value ``value`` (clipped to [-1, 1]): ``"anchored"`` =
        log(prior[i]) + value / anchor, the KL-anchored best response layer 1 makes (layer 1's prior and anchor);
        ``"q"`` (or anchor 0) = the value."""
        value = float(np.clip(value, -1.0, 1.0))
        if self.pick == "anchored" and self.cfg.anchor > 0:
            return float(self.log_prior[i]) + value / self.cfg.anchor
        return value

    # -- driver
    def run(self) -> dict:
        l1, cfg2 = self.l1, self.cfg2
        x1, q1 = np.asarray(l1.x, dtype=float), np.asarray(l1.q, dtype=float)
        layer1_choice = int(np.argmax(x1))
        order = [int(i) for i in np.argsort(-x1, kind="stable") if x1[i] > 0]
        shortlist = order[:max(cfg2.shortlist, 1)]
        allowed = self.legal.any(0)
        if allowed.any():
            qbest = int(np.argmax(np.where(allowed, q1, -np.inf)))
            if qbest not in shortlist:
                shortlist.append(qbest)
        info = dict(shortlist=shortlist, layer1_choice=layer1_choice, rounds=[], completed=0, skipped=None)
        if len(shortlist) < 2:
            info["skipped"] = "one candidate"
            return info
        score = {i: self.score(i, q1[i]) for i in shortlist}
        survivors = list(shortlist)
        for r, rnd in enumerate(cfg2.rounds):
            if r > 0 or rnd.keep < cfg2.shortlist:
                ranked = sorted(survivors, key=lambda i: (-score[i], survivors.index(i)))
                survivors = ranked[:max(rnd.keep, 1)]
            worlds = [int(k) for k in self.perm[:max(min(rnd.worlds, self.K), 1)]]
            chance = max(rnd.chance, 1)
            keys = [(k, i, j, c) for i in survivors for k in worlds if self.legal[k][i]
                    for j, _ in self.reply_set(k, i) for c in range(chance)]
            new = [key for key in keys if key not in self.leaves]
            if not self.grow(new) or not self.layer2(keys):
                info["skipped"] = self.clock.refused or "stopped"
                break
            Q2, new_score = self.backup(survivors, worlds, chance)
            score.update(new_score)
            info["rounds"].append(dict(survivors=list(survivors), worlds=len(worlds), chance=chance,
                                       leaves=len(keys), q2=Q2, score={i: new_score[i] for i in survivors}))
            info["completed"] = r + 1
        if info["completed"]:
            last = info["rounds"][-1]
            info["chosen"] = max(last["survivors"], key=lambda i: (last["score"][i], -last["survivors"].index(i)))
        return info


def deepen(bot, cfg, cfg2: Depth2Config, layer1: Layer1, battle=None, deadline: float | None = None, rng=None,
           budget=None, describe=None, pick: str | None = None, costs: dict | None = None,
           threat_reply: bool | None = None) -> dict | None:
    """The second layer for one decision (module docstring). Returns None when there is nothing to deepen, else a
    dict: ``chosen`` (index into ``layer1.ours``, or None to keep layer 1), ``q2`` {index: Q2}, ``cells``,
    ``leaves``, ``stages`` (list of ``Stage``), ``changed``, ``rounds`` completed and ``analysis`` (the compact,
    JSON-serialisable ``depth2`` block). ``describe(a0, a1)`` names a pair for the analysis; ``battle`` (the
    player's ``_Battle``) names our active Pokemon in opponent replies; ``costs`` keeps per-unit stage costs across
    decisions for deadline checks without a budget controller; ``budget`` (a BudgetController) admits every chunk
    when given; ``pick`` and ``threat_reply`` override ``cfg2.pick`` / ``cfg2.threat_reply``."""
    started = time.perf_counter()
    clock = StageClock(budget, deadline, costs)
    ours_shown = ("", "")
    if battle is not None:
        ours_shown = tuple(battle.active.get(("p1", "p2")[layer1.side] + slot, "") for slot in "ab")
    run = _Deepen(bot, cfg, cfg2, layer1, clock, rng if rng is not None else bot.rng, pick or cfg2.pick, ours_shown,
                  describe, cfg2.threat_reply if threat_reply is None else threat_reply)
    info = run.run()
    chosen = info.get("chosen")
    layer1_choice = info["layer1_choice"]
    leaves = list(run.leaves.values())
    out = dict(chosen=chosen, q2=info["rounds"][-1]["q2"] if info["rounds"] else {},
               cells=dict(run.cells), leaves=len(leaves), stages=list(clock.stages), rounds=info["completed"],
               changed=chosen is not None and chosen != layer1_choice)
    out["analysis"] = _analysis(run, info, out, clock, describe, (time.perf_counter() - started) * 1000)
    return out


def _analysis(run: _Deepen, info: dict, out: dict, clock: StageClock, describe, total_ms: float) -> dict:
    l1 = run.l1
    name = describe or (lambda a0, a1: f"{a0},{a1}")
    candidates = []
    for i in info["shortlist"]:
        a0, a1 = (int(v) for v in l1.ours[i])
        q2 = [round(float(r["q2"][i]), 4) if i in r["q2"] else None for r in info["rounds"]]
        candidates.append(dict(choice=name(a0, a1), x=round(float(l1.x[i]), 4),
                               prior=round(float(l1.our_prior[i]), 6), q=round(float(l1.q[i]), 4),
                               q2=q2, score=[round(float(r["score"][i]), 3) if i in r["score"] else None
                                             for r in info["rounds"]]))
    # Replies deepened per candidate (merged over worlds by what we would see), with their layer-1 weights.
    replies = []
    for i in info["shortlist"]:
        merged: dict = {}
        worlds = {k for (k, ii) in run.replies if ii == i}
        for (k, ii), reps in run.replies.items():
            if ii != i:
                continue
            for j, danger in reps:
                text = run.obs_text(k, j)
                e = merged.setdefault(text, dict(text=text, weight=0.0, worlds=0, danger=False))
                e["weight"] += float(l1.ys[k][j]) / max(len(worlds), 1)
                e["worlds"] += 1
                e["danger"] = e["danger"] or danger
        top = sorted(merged.values(), key=lambda e: -e["weight"])[:4]
        for e in top:
            e["weight"] = round(e["weight"], 3)
        replies.append(top)
    leaves = list(run.leaves.values())
    ended = sum(lf.ended for lf in leaves)
    failed = sum(bool(lf.failed) for lf in leaves)
    chosen = info.get("chosen")
    return dict(
        rounds=info["completed"], rounds_planned=len(run.cfg2.rounds),
        sizes=[dict(keep=len(r["survivors"]), worlds=r["worlds"], chance=r["chance"], leaves=r["leaves"])
               for r in info["rounds"]],
        candidates=candidates, replies=replies,
        chosen=None if chosen is None else name(*(int(v) for v in l1.ours[chosen])),
        changed=bool(out["changed"]), skipped=info.get("skipped"),
        cells=dict(run.cells), leaves=dict(total=len(leaves), ended=ended, failed=failed,
                                           forced_steps=run.forced_steps),
        timing={k: v["ms"] for k, v in clock.totals().items()}, units={k: v["units"] for k, v in clock.totals().items()},
        total_ms=round(total_ms, 1),
        config=dict(backup=run.cfg2.backup, info=run.cfg2.info, leaf_value=run.cfg2.leaf_value, pick=run.pick,
                    threat_reply=run.threat_reply, common_opponent=run.common_opponent))
