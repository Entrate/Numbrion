"""One-turn search over sampled opponent worlds, on top of the self-play policy and its value head.

Per decision of a protocol-driven player (training.player):

1. Root: the policy on our observation gives our candidate pairs (the most likely joint actions) and their prior.
2. Worlds: ``numbrion.WorldBuilder`` samples opponent teams consistent with everything we have seen and replays the
   battle to the current boundary (fair information: it reads only our lines, requests and choices).
3. Opponent: in every world the policy, run from the opponent's side, gives the opponent's candidate pairs and prior.
   Its observation is approximated by a tracker fed with *our* player-view lines from the opponent's side (the two
   views differ only in exact HP of each side's own Pokemon) plus the world's request for the opponent.
4. Grid: every (world, our pair, their pair, chance seed) cell is simulated to the next decision boundary. Cells of
   one world and seed share the random numbers, so pairs are compared under the same rolls.
5. Leaves are scored with the value head (an oracle critic: it reads the other side's own team, which every world
   supplies). The critic only trained on rows whose side had to act, so a leaf where only the opponent must act
   (a replacement after a faint) is scored from the opponent's side and negated. Ended battles score +1/-1/0.
6. The payoff matrices form a Bayesian matrix game: our mixed strategy is shared by all worlds (we do not know which
   one is true), the opponent's is per world. Smoothed fictitious play with KL anchors to both priors solves it
   (``opponent_anchor`` 0 instead makes the opponent play its prior: a best response to the policy). We play the
   pair our strategy weights most; sampling from it was clearly weaker against the policy (docs/play/SEARCH.md).

When no world reproduces the observations, or the decision has only one legal pair, the player plays the policy.
"""
from __future__ import annotations

import contextlib
import json
import threading
import time
import warnings
from dataclasses import asdict, dataclass, field, replace
from pathlib import Path

import numpy as np
import torch

from .dex import Dex, load_dex
from .encoder import Buffers
from .guards import MoveGuards
from .model import Model
from .player import Bot, Decision, Player
from .protocol import LineNormalizer, choice_text, codes_from_choice, request_mask, team_from_request

N_ACTIONS = 47
PASS = 46
OBS_KEYS = ("ids", "floats", "field", "action_features", "token_features")
STATE_SHARED = ("tables",)
REPO = Path(__file__).resolve().parents[3]
DEADLINE_MARGIN_MS = 150.0  # with a deadline: time kept for solving, the analysis and sending after the cells

warnings.filterwarnings("ignore", message=r"numbrion\.\w+: search prerequisites")


@dataclass
class SearchConfig:
    worlds: int = 4                 # sampled opponent teams per decision
    ours: int = 8                   # our candidate pairs
    theirs: int = 6                 # opponent candidate pairs per world
    seeds: int = 2                  # chance samples per cell, shared across pairs (common random numbers)
    first_codes: int = 47           # slot-a codes expanded when enumerating pairs (47: every legal one)
    prior_temperature: float = 1.0  # our prior (anchor) is the policy at this temperature
    opponent_temperature: float = 1.0
    anchor: float = 0.05            # KL weight on our prior, in value units ([-1, 1] payoffs)
    opponent_anchor: float = 0.1    # KL weight on the opponent's prior; 0 = it just plays the prior
    iterations: int = 200
    play: str = "argmax"            # "argmax" of our strategy or "sample" from it
    particles: int = 8              # WorldBuilder particles (16 doubled world-building time)
    threads: int = 4                # grid threads
    seed: int = 0
    max_worlds: int = 32            # budgeted decisions: budget_ms scales the worlds between 2 and this

    @classmethod
    def from_dict(cls, values: dict | None) -> "SearchConfig":
        known = {k: v for k, v in (values or {}).items() if k in cls.__dataclass_fields__}
        return cls(**known)


# ---- network -------------------------------------------------------------------------------------------------------

def _take(state: dict, index: torch.Tensor) -> dict:
    return {k: (v if k in STATE_SHARED else v.index_select(0, index)) for k, v in state.items()}


def _log_softmax(logits: np.ndarray, mask: np.ndarray, temperature: float) -> np.ndarray:
    z = np.where(mask, logits / max(temperature, 1e-6), -np.inf)
    z = z - z.max(-1, keepdims=True)
    return z - np.log(np.exp(z).sum(-1, keepdims=True))


_GPU_LOCK = threading.Lock()


@torch.no_grad()
def pair_candidates(model: Model, state: dict, guard: np.ndarray, masks: list, k: int, first: int,
                    temperature: float, guards: MoveGuards, lock=None):
    """``Net.candidates`` from trunk outputs ``state`` (one row per entry of ``masks``; ``guard`` their guard
    arrays). ``lock`` (optional) is held around the model calls."""
    lock = lock or contextlib.nullcontext()
    n = len(masks)
    mask0 = np.stack([m[0] for m in masks])
    mask0, _ = guards.apply(mask0, guard, 0)
    with lock:
        logits0, vectors0 = model.logits(state, 0)
        logits0 = logits0.numpy()
    lp0 = _log_softmax(logits0, mask0, 1.0)
    lp0_t = _log_softmax(logits0, mask0, temperature)
    expand_rows, expand_a0 = [], []
    for i in range(n):
        legal = np.flatnonzero(mask0[i])
        order = legal[np.argsort(-lp0[i, legal], kind="stable")][:first]
        expand_rows += [i] * len(order)
        expand_a0 += order.tolist()
    expand_rows = np.asarray(expand_rows, dtype=np.int64)
    expand_a0 = np.asarray(expand_a0, dtype=np.int64)
    if not len(expand_rows):
        return [(np.zeros((0, 2), np.int32), np.zeros(0, np.float32), {}) for _ in range(n)]
    mask1 = np.stack([masks[i][1][a] for i, a in zip(expand_rows, expand_a0)])
    mask1, _ = guards.apply(mask1, guard[expand_rows], 1)
    with lock:
        index = torch.from_numpy(expand_rows)
        sub = _take(state, index)
        a0 = torch.from_numpy(expand_a0)
        onehot = (a0.view(-1, 1) == torch.arange(N_ACTIONS)).float()
        chosen = (onehot.unsqueeze(-1) * vectors0.index_select(0, index)).sum(1)
        joint = model.act_joint(model.partner_features(sub, onehot)) if model.joint_features else None
        logits1, _ = model.logits(sub, 1, chosen, joint)
        logits1 = logits1.numpy()
    lp1 = _log_softmax(logits1, mask1, 1.0)
    lp1_t = _log_softmax(logits1, mask1, temperature)
    out = [([], [], {}) for _ in range(n)]
    for e, (i, a0v) in enumerate(zip(expand_rows, expand_a0)):
        legal = np.flatnonzero(mask1[e])
        pairs, priors, full = out[i]
        for a1v in legal:
            pairs.append((a0v, a1v))
            priors.append(lp0_t[i, a0v] + lp1_t[e, a1v])
            full[(int(a0v), int(a1v))] = (float(lp0[i, a0v] + lp1[e, a1v]), float(lp0_t[i, a0v] + lp1_t[e, a1v]))
    result = []
    for pairs, priors, full in out:
        pairs, priors = np.asarray(pairs, dtype=np.int32).reshape(-1, 2), np.asarray(priors, dtype=np.float32)
        order = np.argsort(-priors, kind="stable")[:k]
        result.append((pairs[order], priors[order], full))
    return result


class Net:
    """The policy/value network for search: batched candidate enumeration and leaf values."""

    def __init__(self, checkpoint: Path, dex: Dex, device: str = "auto"):
        if device == "auto":  # the Apple GPU evaluates search batches ~1.5-2x faster than the CPU and frees it
            device = "mps" if torch.backends.mps.is_available() else "cpu"
        if device == "directml":  # Windows AMD/Intel GPUs (training.device); leaf batches only, like MPS
            from .device import training_device
            device = training_device("directml")[0]
        state = torch.load(checkpoint, map_location="cpu", weights_only=False)
        self.width, self.layers = state.get("width", 128), state.get("layers", 3)
        # Policy calls are a few rows each, where the GPU's per-call latency dominates: they use a CPU copy. Leaf
        # values (hundreds of rows) use ``device``.
        self.policy_model = Model.from_state(dex, state["model"], self.width, self.layers).eval()
        self.model = Model.from_state(dex, state["model"], self.width, self.layers).to(device).eval()
        self.device = torch.device(device)
        self.guards = MoveGuards()
        # Every Net of the process shares one GPU lock (MPS calls from several bots' threads are serialised).
        self.lock = _GPU_LOCK if self.device.type != "cpu" else threading.Lock()
        self.policy_lock = threading.Lock()


    @torch.no_grad()
    def candidates(self, arrays: dict, rows: np.ndarray, masks: list, k: int, first: int, temperature: float):
        """Top-``k`` joint pairs for each of ``rows`` of ``arrays``, with their log-prior at ``temperature``.

        ``masks[i]`` = (mask0 [47], joint [47, 47]) of row i. Guards (training.guards) apply as for the base policy.
        Returns per row (pairs int32 [m, 2], log_prior float32 [m], full): the top pairs, most likely first, and
        ``full`` = {(a0, a1): (log-prob at T=1, log-prob at ``temperature``)} for every enumerated legal pair (slot a
        restricted to its ``first`` most likely codes). Tempering is per slot, as the base policy samples."""
        model = self.policy_model
        with self.policy_lock:
            x = {k: torch.from_numpy(np.ascontiguousarray(arrays[k][rows])) for k in OBS_KEYS}
            state = model.trunk(x["ids"], x["floats"], x["field"], x["action_features"], x["token_features"])
        return pair_candidates(model, state, arrays["guard"][rows], masks, k, first, temperature, self.guards,
                               self.policy_lock)

    @torch.no_grad()
    def values(self, arrays: dict, partner_ids: np.ndarray, partner_floats: np.ndarray, chunk: int = 2048):
        """Value head for each row of ``arrays`` (ids, floats, field, token_features) against the given partner (the
        other side's own-team tokens 0-5)."""
        n = len(arrays["field"])
        out = np.empty(n, dtype=np.float32)
        up = lambda a: torch.from_numpy(np.ascontiguousarray(a)).to(self.device)  # noqa: E731
        with self.lock:
            for begin in range(0, n, chunk):
                rows = slice(begin, min(n, begin + chunk))
                count = rows.stop - rows.start
                no_actions = torch.zeros(count, 2, 40, 0, device=self.device)  # the value head never reads them
                state = self.model.trunk(up(arrays["ids"][rows]), up(arrays["floats"][rows]), up(arrays["field"][rows]),
                                         no_actions, up(arrays["token_features"][rows]))
                out[rows] = self.model.value(state, up(partner_ids[rows]), up(partner_floats[rows])).cpu().numpy()
        return out


# ---- matrix game ---------------------------------------------------------------------------------------------------

def _softmax(z: np.ndarray) -> np.ndarray:
    z = z - z.max()
    e = np.exp(z)
    return e / e.sum()


def solve(payoffs: list, valid: list, world_weights: np.ndarray, our_prior: np.ndarray, their_priors: list,
          anchor: float, opponent_anchor: float, iterations: int):
    """Smoothed fictitious play on the Bayesian matrix game.

    ``payoffs[w]`` [N, M_w]: our expected value; ``valid[w]`` [N] bool: our pair is legal in world w. Our strategy
    x (shared by the worlds) best-responds to the averaged opponent strategies with a KL anchor ``anchor`` to
    ``our_prior``; each world's opponent strategy y_w minimises our payoff with anchor ``opponent_anchor`` to its
    prior (0: it plays the prior). Returns (x, ys, q) with q[i] our expected value of pair i against ys."""
    allowed = np.any(valid, axis=0)
    our_prior = np.where(allowed, our_prior, 0.0)
    our_prior = our_prior / max(our_prior.sum(), 1e-12)
    log_x0 = np.where(allowed, np.log(np.maximum(our_prior, 1e-300)), -np.inf)  # forbidden pairs get no weight
    log_y0 = [np.log(np.maximum(p, 1e-12)) for p in their_priors]
    ys = [p.copy() for p in their_priors]
    x_avg = our_prior.copy()
    w = world_weights / world_weights.sum()

    def our_values(ys):
        total = np.zeros(len(our_prior))
        weight = np.zeros(len(our_prior))
        for k, (a, ok) in enumerate(zip(payoffs, valid)):
            total += np.where(ok, w[k] * (a @ ys[k]), 0.0)
            weight += np.where(ok, w[k], 0.0)
        # A pair no sampled world allows (e.g. a switch a possible Shadow Tag would block) scores below any outcome.
        return np.where(weight > 0, total / np.maximum(weight, 1e-12), -2.0)

    def respond(q):
        if anchor > 0:
            return _softmax(log_x0 + q / anchor)
        best = (q == q[allowed].max()) & allowed
        return best / best.sum()

    if opponent_anchor <= 0:
        q = our_values(ys)
        return respond(q), ys, q
    y_avg = [y.copy() for y in ys]
    for t in range(1, iterations + 1):
        q = our_values(y_avg)
        x = respond(q)
        x_avg += (x - x_avg) / (t + 1)
        for k, (a, ok) in enumerate(zip(payoffs, valid)):
            xs = np.where(ok, x_avg, 0.0)
            xs = xs / max(xs.sum(), 1e-12)
            y = _softmax(log_y0[k] - (xs @ a) / opponent_anchor)
            y_avg[k] += (y - y_avg[k]) / (t + 1)
    return x_avg, y_avg, our_values(y_avg)


def solve_batch(payoffs: np.ndarray, our_prior: np.ndarray, their_prior: np.ndarray, anchor: float,
                opponent_anchor: float, iterations: int):
    """``solve`` for B independent one-world games at once (the in-training search opponent).

    ``payoffs`` [B, N, M]; ``our_prior`` [B, N] and ``their_prior`` [B, M] are probabilities, 0 for padding
    entries (which then get no weight; padded payoffs are ignored). Every unpadded pair is valid. Returns (x [B, N],
    q [B, N]) as ``solve`` does for one world with all pairs valid."""
    ours_ok, theirs_ok = our_prior > 0, their_prior > 0
    our_prior = our_prior / our_prior.sum(1, keepdims=True)
    their_prior = their_prior / their_prior.sum(1, keepdims=True)
    a = np.where(ours_ok[:, :, None] & theirs_ok[:, None, :], payoffs, 0.0)
    with np.errstate(divide="ignore"):
        log_x0 = np.where(ours_ok, np.log(np.maximum(our_prior, 1e-300)), -np.inf)
        log_y0 = np.where(theirs_ok, np.log(np.maximum(their_prior, 1e-12)), -np.inf)

    def values(y):
        return np.where(ours_ok, np.einsum("bnm,bm->bn", a, y), -2.0)

    def softmax(z):
        z = z - z.max(1, keepdims=True)
        e = np.exp(z)
        return e / e.sum(1, keepdims=True)

    def respond(q):
        if anchor > 0:
            return softmax(log_x0 + q / anchor)
        best = (q == np.where(ours_ok, q, -np.inf).max(1, keepdims=True)) & ours_ok
        return best / best.sum(1, keepdims=True)

    y_avg = their_prior.copy()
    if opponent_anchor <= 0:
        q = values(y_avg)
        return respond(q), q
    x_avg = our_prior.copy()
    for t in range(1, iterations + 1):
        x = respond(values(y_avg))
        x_avg += (x - x_avg) / (t + 1)
        y = softmax(log_y0 - np.einsum("bn,bnm->bm", x_avg, a) / opponent_anchor)
        y_avg += (y - y_avg) / (t + 1)
    return x_avg, values(y_avg)


# ---- players -------------------------------------------------------------------------------------------------------

def describe_code(request: dict, slot: int, code: int, foes: tuple = ("foe a", "foe b")) -> str:
    """Readable text of one slot's code from our request (``"Spore -> Incineroar"``); ``foes`` names the foe slots."""
    side = request.get("side", {}).get("pokemon", [])
    if code == PASS:
        return "pass"
    if code >= 40:
        p = code - 40
        name = side[p]["details"].split(",")[0] if p < len(side) else f"party {p + 1}"
        return f"switch to {name}"
    move_slot, rest = divmod(code, 10)
    target, tera = divmod(rest, 2)
    active = request.get("active") or []
    moves = active[slot].get("moves", []) if slot < len(active) else []
    name = moves[move_slot]["move"] if move_slot < len(moves) else f"move {move_slot + 1}"
    if target == 2:
        where = ""
    elif target >= 3:
        where = f" -> {foes[target - 3] or ('foe a', 'foe b')[target - 3]}"
    else:  # code 0 = own slot b, 1 = own slot a
        where = " -> self" if (1 - target) == slot else " -> ally"
    return f"{name}{where}{' + Tera' if tera else ''}"


HP_LINES = {"switch": 4, "drag": 4, "-damage": 3, "-heal": 3, "-sethp": 3}  # command -> condition field


def public_hp(lines: list[str], side: int) -> list[str]:
    """``lines`` with side ``side``'s exact HP replaced by what the other player sees (Showdown's percentage: ceil of
    the ratio, 99 below full), so a tracker fed our lines from the opponent's side does not know our exact HP."""
    prefix = "p1" if side == 0 else "p2"
    out = []
    for line in lines:
        parts = line.split("|")
        field = HP_LINES.get(parts[1]) if len(parts) > 1 else None
        if field is not None and len(parts) > field and parts[2].startswith(prefix):
            hp, _, status = parts[field].partition(" ")
            current, _, top = hp.partition("/")
            if top and top != "100" and current.isdigit() and top.isdigit() and int(top) > 0:
                ratio = int(current) / int(top)
                percent = -(-int(current) * 100 // int(top))
                if percent == 100 and ratio < 1:
                    percent = 99
                parts[field] = f"{percent}/100" + (f" {status}" if status else "")
                line = "|".join(parts)
        out.append(line)
    return out


class _Battle:
    """Protocol state of one battle shared by policy and search players: the encoder rows of both sides."""

    def __init__(self, dex: Dex):
        from numbrion import ObsEncoder

        self.encoder = ObsEncoder(str(dex.path), 2)
        self.buffers = Buffers(2)
        self.active: dict[str, str] = {}  # "p2a" -> species shown there (for readable analysis only)

    def outputs(self):
        b = self.buffers
        return (b.ids, b.floats, b.field, b.foe_match, b.fainted, b.guard, b.action_features, b.token_features)

    def update(self, side: int, request_json: str, lines: list[str]) -> None:
        """Feed ``lines`` to both trackers; encode our row with ``request_json`` (row ``side``)."""
        for line in lines:
            parts = line.split("|")
            if len(parts) > 3 and parts[1] in ("switch", "drag", "replace"):
                self.active[parts[2][:3]] = parts[3].split(",")[0]
        self.encoder.encode_rows([side], [dict(battle_id=1, request=request_json, log=lines)], *self.outputs())
        self.encoder.encode_rows([1 - side], [dict(battle_id=1, request=None, log=public_hp(lines, side))],
                                 *self.outputs(), damage=False)

    def arrays(self) -> dict:
        b = self.buffers
        return dict(ids=b.ids, floats=b.floats, field=b.field, action_features=b.action_features,
                    token_features=b.token_features, guard=b.guard)


_POOLS: dict = {}
_POOLS_LOCK = threading.Lock()


class SearchBot(Bot):
    """A checkpoint plus search settings; makes SearchPlayers (search off: policy players)."""

    def __init__(self, name: str = "Numbrion", checkpoint: str = "", search: dict | bool | None = None,
                 pools: list | None = None, device: str = "auto", policy_temperature: float = 0.5,
                 torch_threads: int | None = None, cell_ms: float = 0.3, **_):
        self.name = name
        self.dex = load_dex()
        if torch_threads:
            torch.set_num_threads(torch_threads)
        self.checkpoint = Path(checkpoint)
        self.net = Net(self.checkpoint, self.dex, device)
        # search: a SearchConfig dict, True/None for the defaults, False for the policy alone.
        self.search = None if search is False else SearchConfig.from_dict(search if isinstance(search, dict) else {})
        self.policy_temperature = policy_temperature
        self.pool_paths = [str(REPO / p) if not Path(p).is_absolute() else p
                           for p in (pools or ["data/teams/pool-s1-200k.txt.gz"])]
        self._pool = None
        self._local = threading.local()
        self.rng = np.random.default_rng(self.search.seed if self.search else 0)
        # Running cost estimates for budgeted decisions: ms per simulated cell, and ms per decision outside cells.
        # ``cell_ms`` is the starting guess (0.3 fits 128x3; larger nets cost more until the first decisions measure).
        self.cell_ms, self.fixed_ms = cell_ms, 60.0

    def config_for(self, budget_ms: float | None) -> "SearchConfig":
        """The search config with as many worlds as fit ``budget_ms`` (at least 2, at most ``max_worlds``)."""
        cfg = self.search
        if not budget_ms:
            return cfg
        per_world = cfg.ours * cfg.theirs * cfg.seeds * self.cell_ms
        worlds = int((budget_ms - self.fixed_ms) // max(per_world, 1e-3))
        return replace(cfg, worlds=max(2, min(cfg.max_worlds, worlds)))

    def observe_cost(self, timing: dict, cells: int, total_ms: float) -> None:
        cell_part = sum(timing.get(k, 0.0) for k in ("opponent_ms", "grid_ms", "value_ms"))
        if cells:
            self.cell_ms += 0.2 * (cell_part / cells - self.cell_ms)
        self.fixed_ms += 0.2 * (max(total_ms - cell_part, 0.0) - self.fixed_ms)

    @property
    def pool(self):
        """The opponent-set pool, shared by every bot of the process that uses the same files (~300 MB each)."""
        if self._pool is None:
            from numbrion import SetPool

            with _POOLS_LOCK:
                key = tuple(self.pool_paths)
                if key not in _POOLS:
                    _POOLS[key] = SetPool(self.pool_paths)
                self._pool = _POOLS[key]
        return self._pool

    def resources(self):
        """Per-thread search grid (engine cells and leaf encoding on Rust threads)."""
        local = self._local
        if not hasattr(local, "grid"):
            import numbrion

            local.grid = numbrion.SearchGrid(str(self.dex.path), self.search.threads if self.search else 1)
        return local

    def warmup(self) -> None:
        """Load what the first decision would (the team pool, the grid threads) ahead of time."""
        if self.search is not None:
            _ = self.pool
            self.resources()

    def new_player(self, room: str) -> Player:
        return SearchPlayer(self, room)

    def describe(self) -> dict:
        return dict(name=self.name, checkpoint=str(self.checkpoint), width=self.net.width, layers=self.net.layers,
                    search=asdict(self.search) if self.search else None, policy_temperature=self.policy_temperature)


class SearchPlayer(Player):
    """Protocol-driven player: search when worlds are available, else the policy at ``policy_temperature``.

    ``team`` (packed, genders resolved) is our own team; without it the first request is used
    (training.protocol.team_from_request). ``mask_hint`` = (mask0, joint) may be set by an in-process driver that
    owns the battle, replacing the request-derived mask (debugging only)."""

    def __init__(self, bot: SearchBot, room: str = "", team: str | None = None):
        super().__init__()
        self.bot, self.room = bot, room
        self.team = team
        self.battle = _Battle(bot.dex)
        self.builder = None
        self.builder_failed = ""
        self.fed = 0
        self.normalize = LineNormalizer()  # Showdown room lines -> engine player-view lines (stateful)
        self.seen: list[str] = []  # normalized lines so far (for the team's start-of-battle reveals)
        self.pending_choice: str | None = None  # last choice text, pushed to the world builder with the next lines
        self.pending_decided = False  # pending_choice came from decide (a refusal drops it), not record_choice
        self.synced = False  # a request has been synced (later ones without new lines are re-sends)
        self.mask_hint = None
        self.budget_ms: float | None = None  # per-decision time budget (scales the number of worlds)
        # Set by a driver with a deadline (play.driver): the time.monotonic() by which the choice must be sent, and a
        # token for this decision. ``fallback`` = (token, Decision) is the policy's choice, posted as soon as it is
        # known so the driver can send it if the search runs late.
        self.deadline: float | None = None
        self.decision_token = None
        self.fallback: tuple | None = None
        self.decisions = 0
        self.stats = dict(search=0, policy=0, forced=0, no_worlds=0)
        # The last decision's search (None if the policy decided): what search_labels.py stores for distillation.
        self.last_search: dict | None = None

    # -- protocol bookkeeping
    def _ensure_builder(self, request: dict, side: int) -> None:
        if self.builder is not None or self.bot.search is None or self.builder_failed:
            return
        from numbrion import WorldBuilder

        team = self.team
        try:
            if team is None:
                team = team_from_request(request, self.seen)
            self.builder = WorldBuilder(side, team, self.bot.pool, seed=int(self.bot.rng.integers(1 << 62)),
                                        particles=self.bot.search.particles)
        except Exception as error:  # noqa: BLE001 - search is optional; the policy still plays
            self.builder_failed = f"world builder unavailable: {error}"

    def _sync(self, request: dict, side: int) -> list[str]:
        new = self.normalize(self.lines[self.fed:])
        self.fed = len(self.lines)
        if self.synced and not new:
            # The same boundary again: a request re-sent after a refused choice. Showdown sends no other request
            # without new lines. The world builder keeps the first request: its worlds never made the refused
            # attempt, so they cannot reproduce what the refusal revealed (trapped, disabled moves). Our row is
            # re-encoded with the new request, and the refused choice is not replayed.
            self.battle.update(side, self.request_json, new)
            return new
        if self.builder is None and len(self.seen) < 2000:
            self.seen.extend(new)
        self._ensure_builder(request, side)
        if self.builder is not None:
            if self.pending_choice is not None:
                self.builder.push_choice(self.pending_choice)
            self.builder.push_lines(new)
            self.builder.push_request(self.request_json)
        self.pending_choice = None
        self.pending_decided = False
        self.synced = True
        self.battle.update(side, self.request_json, new)
        return new

    def rejected(self, message: str) -> None:
        if self.pending_decided:
            self.pending_choice = None

    def record_choice(self, choice: str) -> None:
        request = json.loads(self.request_json) if self.request_json else {}
        if not request or request.get("wait") or request.get("teamPreview"):
            return
        side = 0 if request["side"]["id"] == "p1" else 1
        self.side = side
        self._sync(request, side)
        try:  # the canonical text of the same choice ("move Moonblast 1" -> "move 1 +1")
            text = choice_text(request, *codes_from_choice(request, choice)) or choice
        except ValueError:
            text = choice
        self.pending_choice = text
        self.pending_decided = False

    def _mask(self, request: dict):
        if self.mask_hint is not None:
            return self.mask_hint
        return request_mask(request)

    # -- decisions
    def decide(self) -> Decision | None:
        self.last_search = None
        request = json.loads(self.request_json) if self.request_json else {}
        if not request or request.get("wait") or request.get("teamPreview"):
            return None
        side = 0 if request["side"]["id"] == "p1" else 1
        self.side = side
        started = time.perf_counter()
        self._sync(request, side)
        self.stats["sync_ms"] = self.stats.get("sync_ms", 0.0) + (time.perf_counter() - started) * 1000
        mask0, joint = self._mask(request)
        legal = np.argwhere(joint)
        if len(legal) == 0:
            return None
        self.decisions += 1
        arrays = self.battle.arrays()
        if len(legal) == 1:
            a0, a1 = map(int, legal[0])
            self.stats["forced"] += 1
            return self._finish(request, a0, a1, dict(source="policy", note="only one legal choice"), started)
        cfg = self.bot.config_for(self.budget_ms) if self.bot.search else None
        k = cfg.ours if cfg else 1
        first = cfg.first_codes if cfg else 47
        t_root = time.perf_counter()
        (pairs, prior, full), = self.bot.net.candidates(arrays, np.array([side]), [(mask0, joint)], k=max(k, 1),
                                                       first=first, temperature=self.bot.policy_temperature)
        self.stats["root_ms"] = self.stats.get("root_ms", 0.0) + (time.perf_counter() - t_root) * 1000
        policy = self._post_fallback(request, full)
        if cfg is None or self.builder is None:
            return self._policy(request, full, started, note=self.builder_failed or "search off")
        t_worlds = time.perf_counter()
        try:
            worlds = self.builder.worlds(cfg.worlds, int(self.bot.rng.integers(1 << 62)), log=True)
        except RuntimeError as error:
            self.stats["no_worlds"] += 1
            return self._policy(request, full, started, note=f"no world: {str(error)[:120]}")
        if len(pairs) < 2:
            a0, a1 = map(int, pairs[0])
            return self._finish(request, a0, a1, dict(source="policy", note="one candidate"), started)
        worlds_ms = (time.perf_counter() - t_worlds) * 1000
        requested = len(worlds)
        if self.deadline is not None:
            # Keep the worlds whose cells fit the time left (the bot's running cost per cell), or play the policy.
            left_ms = (self.deadline - time.monotonic()) * 1000 - DEADLINE_MARGIN_MS
            per_world = cfg.ours * cfg.theirs * cfg.seeds * self.bot.cell_ms
            fit = int(left_ms // max(per_world, 1e-3))
            if fit < 1:
                self.stats["out_of_time"] = self.stats.get("out_of_time", 0) + 1
                return self._policy(request, full, started, note="out of time after building the worlds")
            worlds = worlds[:fit]
        result = run_search(self.bot, cfg, self.battle, side, worlds, pairs, full)
        if result is None:
            return self._policy(request, full, started, note="no candidate is legal in the sampled worlds")
        result["timing"]["worlds_ms"] = round(worlds_ms, 1)
        for key, ms in result["timing"].items():
            self.stats[key] = self.stats.get(key, 0.0) + ms
        self.stats["cells"] = self.stats.get("cells", 0) + result["cells"]
        self.stats["search"] += 1
        x, q = result["x"], result["q"]
        self.last_search = dict(side=side, arrays=arrays, mask0=mask0, joint=joint, pairs=pairs, x=x, q=q, full=full)
        if cfg.play == "sample":
            best = int(self.bot.rng.choice(len(x), p=x / x.sum()))
        else:
            best = int(np.argmax(x))
        a0, a1 = map(int, pairs[best])
        analysis = dict(source="search", value=round(float(q[best]), 4),
                        win_probability=round(float((q[best] + 1) / 2), 3), worlds=len(worlds),
                        worlds_requested=requested,
                        budget_ms=None if self.budget_ms is None else round(self.budget_ms),
                        cells=result["cells"], policy=policy,
                        agrees_with_policy=policy["choice"] == self._text(request, a0, a1),
                        candidates=[dict(choice=self._text(request, int(p[0]), int(p[1])),
                                         text=self._readable(request, int(p[0]), int(p[1])),
                                         prior=round(float(np.exp(full.get((int(p[0]), int(p[1])), (-np.inf,))[0])), 4),
                                         value=round(float(qq), 4), weight=round(float(xx), 4))
                                    for p, qq, xx in sorted(zip(pairs, q, x), key=lambda t: -t[2])],
                        opponent=result["opponent"], timing=result["timing"])
        try:  # what we saw of the opponent's Pokemon, and what the sampled worlds assumed about them
            analysis["observed"], analysis["inferred"] = opponent_knowledge(self.builder.public_state(), worlds, side)
        except Exception as error:  # noqa: BLE001 - analysis only
            analysis["observed"] = {"error": repr(error)[:200]}
        decision = self._finish(request, a0, a1, analysis, started)
        self.bot.observe_cost(result["timing"], result["cells"], analysis["time_ms"])
        return decision

    def _post_fallback(self, request, full) -> dict:
        """The policy's most likely pair: posted as the driver's fallback for this decision (``fallback``), and kept
        in a search decision's analysis (``policy``) to show where search disagrees with the plain policy."""
        (a0, a1), (logp, _) = max(full.items(), key=lambda kv: kv[1][0])
        info = dict(choice=self._text(request, a0, a1), text=self._readable(request, a0, a1),
                    prior=round(float(np.exp(logp)), 4))
        self.fallback = (self.decision_token, Decision(info["choice"], int(a0), int(a1), dict(
            source="policy", note="the policy's choice, sent when search ran out of time", policy=info)))
        return info

    def _policy(self, request, full, started, note=""):
        self.stats["policy"] += 1
        keys = list(full)
        logp = np.array([full[key][0] for key in keys])
        if self.bot.policy_temperature <= 0:
            pick = int(np.argmax(logp))
        else:
            tempered = np.array([full[key][1] for key in keys])
            p = np.exp(tempered - tempered.max())
            pick = int(self.bot.rng.choice(len(keys), p=p / p.sum()))
        a0, a1 = keys[pick]
        top = sorted(full.items(), key=lambda kv: -kv[1][0])[:6]
        return self._finish(request, a0, a1, dict(
            source="policy", note=note,
            candidates=[dict(choice=self._text(request, a, b), text=self._readable(request, a, b),
                             prior=round(float(np.exp(lp[0])), 4)) for (a, b), lp in top]), started)

    def _text(self, request, a0, a1) -> str:
        return choice_text(request, a0, a1) or f"{a0},{a1}"

    def _foes(self) -> tuple:
        foe = "p2" if self.side == 0 else "p1"
        return self.battle.active.get(foe + "a", ""), self.battle.active.get(foe + "b", "")

    def _readable(self, request, a0, a1) -> str:
        foes = self._foes()
        return f"{describe_code(request, 0, a0, foes)}; {describe_code(request, 1, a1, foes)}"

    def _finish(self, request, a0, a1, analysis, started) -> Decision:
        analysis["time_ms"] = round((time.perf_counter() - started) * 1000, 1)
        self.stats["total_ms"] = self.stats.get("total_ms", 0.0) + analysis["time_ms"]
        names = [p["details"].split(",")[0] for p in request.get("side", {}).get("pokemon", [])[:2]]
        foes = self._foes()
        analysis["slots"] = [dict(pokemon=names[i] if i < len(names) else "",
                                  text=describe_code(request, i, c, foes), code=int(c)) for i, c in enumerate((a0, a1))]
        choice = self._text(request, a0, a1)
        self.pending_choice = choice
        self.pending_decided = True
        return Decision(choice, a0, a1, analysis)


def _unpack_sets(packed: str) -> list[dict]:
    """Species, item, ability, moves and Tera type of each Pokemon of a packed team (Showdown's Teams.pack)."""
    sets = []
    for mon in packed.split("]"):
        fields = mon.split("|")
        if len(fields) < 12:
            continue
        misc = fields[11].split(",")
        sets.append(dict(species=fields[1] or fields[0], item=fields[2], ability=fields[3],
                         moves=[m for m in fields[4].split(",") if m], tera=misc[5] if len(misc) > 5 else ""))
    return sets


def _shares(values: list, top: int) -> dict:
    counts: dict = {}
    for value in values:
        counts[value] = counts.get(value, 0) + 1
    n = max(len(values), 1)
    return {k: round(c / n, 2) for k, c in sorted(counts.items(), key=lambda kv: -kv[1])[:top]}


def _id(text: str) -> str:
    return "".join(ch for ch in text.lower() if ch.isalnum())


def opponent_knowledge(public: dict, worlds: list, side: int) -> tuple[list, dict]:
    """(observed, inferred) about the opponent's team. ``observed``: per Pokemon seen, what the protocol revealed
    (``WorldBuilder.public_state``). ``inferred``: per seen species, how often the sampled worlds gave each item,
    ability, Tera type and move, plus the unseen species the worlds put on the bench."""
    opp = 1 - side
    observed = [dict(species=m["species"], moves=m["moves"], item=m["item"], ability=m["ability"],
                     tera_type=m["tera_type"], terastallized=m["terastallized"], fainted=m["fainted"])
                for m in public["sides"][opp]["mons"]]
    seen = {_id(m["species"]) for m in observed}
    by_species: dict = {}
    unseen = []
    for world in worlds:
        for s in _unpack_sets(world.teams[opp]):
            key = _id(s["species"])
            if key in seen:
                by_species.setdefault(key, []).append(s)
            else:
                unseen.append(s["species"])
    inferred = {"worlds": len(worlds), "mons": {}, "unseen_species": _shares(unseen, 8)}
    for sets in by_species.values():
        inferred["mons"][sets[0]["species"]] = dict(
            item=_shares([s["item"] for s in sets], 3), ability=_shares([s["ability"] for s in sets], 3),
            tera=_shares([s["tera"] for s in sets], 3),
            moves=_shares([m for s in sets for m in s["moves"]], 8))
    return observed, inferred


def run_search(bot: SearchBot, cfg: SearchConfig, battle: _Battle, side: int, worlds: list, ours: np.ndarray,
               our_full: dict) -> dict:
    """Steps 3-6 of the module docstring for one decision; returns the strategy and the analysis pieces."""
    net = bot.net
    opp = 1 - side
    timing = {}
    t0 = time.perf_counter()
    grid = bot.resources().grid
    states = (battle.encoder.snapshot(0), battle.encoder.snapshot(1))
    # Opponent candidates per world: the opponent-side tracker with each world's request for the opponent.
    acting = [k for k, world in enumerate(worlds) if world.needs_action(opp)]
    their = [None] * len(worlds)
    if acting:
        encoded = grid.encode_states([states[opp]] * len(acting), [worlds[k].request_json(opp) for k in acting])
        masks = [worlds[k].legal_mask(opp) for k in acting]
        found = net.candidates(encoded, np.arange(len(acting)), masks, cfg.theirs, cfg.first_codes,
                               cfg.opponent_temperature)
        for k, (pairs, logp, _) in zip(acting, found):
            prior = np.exp(logp - logp.max())
            their[k] = (pairs, prior / prior.sum())
    timing["opponent_ms"] = (time.perf_counter() - t0) * 1000
    # Cells: (world, our pair, their pair, seed); one seed per (world, seed index), shared by all pairs.
    t1 = time.perf_counter()
    cells_world, cells_actions, cells_index, seeds = [], [], [], []
    valid = []
    rng = bot.rng
    for k, world in enumerate(worlds):
        world_mask = world.legal_mask(side)[1]
        ok = np.array([bool(world_mask[a, b]) for a, b in ours])
        valid.append(ok)
        opp_pairs = their[k][0] if their[k] is not None else np.full((1, 2), -1, dtype=np.int32)
        for s in range(cfg.seeds):
            seed = rng.integers(0, 1 << 16, size=4)
            for i, pair in enumerate(ours):
                if not ok[i]:
                    continue
                for j, opp_pair in enumerate(opp_pairs):
                    act = np.full((2, 2), -1, dtype=np.int32)
                    act[side] = pair
                    act[opp] = opp_pair
                    cells_world.append(k)
                    cells_actions.append(act)
                    cells_index.append((k, i, j, s))
                    seeds.append(seed)
    n = len(cells_actions)
    if n == 0:
        return None
    out = grid.run(worlds, np.stack(cells_actions), states, side, world=np.asarray(cells_world, dtype=np.int64),
                   seeds=np.stack(seeds).astype(np.int64))
    timing["grid_ms"] = (time.perf_counter() - t1) * 1000
    # Leaf values: from the side that acts at the leaf (ours if we act), negated when it is the opponent's.
    t3 = time.perf_counter()
    eval_side = out["eval_side"]
    live = np.flatnonzero(eval_side >= 0)
    values = np.zeros(n, dtype=np.float32)
    if len(live):
        arrays = {key: out[key][live] for key in ("ids", "floats", "field", "token_features")}
        v = np.clip(net.values(arrays, out["partner_ids"][live], out["partner_floats"][live]), -1.0, 1.0)
        values[live] = np.where(eval_side[live] == side, v, -v)
    done = out["ended"]
    winner = out["winner"]
    values[done] = np.where(winner[done] == side, 1.0, np.where(winner[done] < 0, 0.0, -1.0))
    timing["value_ms"] = (time.perf_counter() - t3) * 1000
    # Payoffs.
    N = len(ours)
    payoffs = []
    for k in range(len(worlds)):
        m = len(their[k][0]) if their[k] is not None else 1
        payoffs.append(np.zeros((N, m)))
    counts = [np.zeros_like(p) for p in payoffs]
    for (k, i, j, s), v in zip(cells_index, values):
        payoffs[k][i, j] += v
        counts[k][i, j] += 1
    payoffs = [p / np.maximum(c, 1) for p, c in zip(payoffs, counts)]
    their_priors = [their[k][1] if their[k] is not None else np.ones(1) for k in range(len(worlds))]
    # A pair that some worlds forbid (a switch a possible Shadow Tag would block) gets refused there, and we choose
    # again: in those worlds it is worth our best allowed pair (against the opponent's prior). Pairs that no world
    # allows stay invalid and get no weight in solve.
    somewhere = np.any(valid, axis=0)
    for k in range(len(worlds)):
        if valid[k].any() and not valid[k][somewhere].all():
            rows = np.flatnonzero(valid[k])
            best = rows[np.argmax(payoffs[k][rows] @ their_priors[k])]
            refused = somewhere & ~valid[k]
            payoffs[k][refused] = payoffs[k][best]
            valid[k] = valid[k] | refused
    logp = np.array([our_full.get((int(a), int(b)), (-30.0,))[0] for a, b in ours]) / max(cfg.prior_temperature, 1e-6)
    our_prior = np.exp(logp - logp.max())
    our_prior /= our_prior.sum()
    x, ys, q = solve(payoffs, valid, np.ones(len(worlds)), our_prior, their_priors, cfg.anchor,
                     cfg.opponent_anchor, cfg.iterations)
    # Opponent replies over worlds, merged by their text: a move's slot (its code) differs between worlds whose sampled
    # sets order the moves differently.
    ours_shown = tuple(battle.active.get(("p1", "p2")[side] + slot, "") for slot in "ab")
    opponent = {}
    for k, world in enumerate(worlds):
        if their[k] is None:
            continue
        request = json.loads(world.request_json(opp))
        names = [p["details"].split(",")[0] for p in request.get("side", {}).get("pokemon", [])[:2]]
        for pair, prior, y in zip(their[k][0], their[k][1], ys[k]):
            codes = (int(pair[0]), int(pair[1]))
            parts = [f"{names[i]}: {describe_code(request, i, c, ours_shown)}" for i, c in enumerate(codes)
                     if c != PASS and i < len(names)]
            text = "; ".join(parts) or "pass"
            entry = opponent.setdefault(text, dict(text=text, prior=0.0, weight=0.0))
            entry["prior"] += float(prior) / len(worlds)
            entry["weight"] += float(y) / len(worlds)
    opponent = sorted(opponent.values(), key=lambda e: -e["weight"])[:6]
    for e in opponent:
        e["prior"], e["weight"] = round(e["prior"], 4), round(e["weight"], 4)
    timing = {k: round(v, 1) for k, v in timing.items()}
    return dict(x=x, q=q, cells=n, opponent=opponent, timing=timing, payoffs=payoffs, valid=valid)
