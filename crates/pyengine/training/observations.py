"""A compact baseline encoder using only own requests and player-view protocol."""
from __future__ import annotations

import hashlib
import json

import numpy as np

FEATURES = 256
STATUSES = ("brn", "par", "slp", "frz", "psn", "tox")


def condition(text: str) -> tuple[float, str]:
    parts = text.split()
    if not parts or "fnt" in parts:
        return 0.0, "fnt"
    hp = parts[0].split("/")
    return (float(hp[0]) / max(float(hp[1]), 1) if len(hp) == 2 else 1.0), (parts[1] if len(parts) > 1 else "")


class PublicEncoder:
    def __init__(self, n_envs: int):
        self.views = [dict(battle_id=-1, mons={}, active={}, fields={}) for _ in range(n_envs * 2)]

    def encode(self, observation: dict, row: int, side: int) -> np.ndarray:
        view = self.views[row]
        if view["battle_id"] != observation["battle_id"]:
            view.update(battle_id=observation["battle_id"], mons={}, active={}, fields={})
        own = f"p{side + 1}"
        for line in observation["log"]:
            self._line(view, line, own)
        request = json.loads(observation["request"] or "{}")
        features = np.zeros(FEATURES, dtype=np.float32)

        def token(text: str) -> None:
            digest = hashlib.blake2b(text.encode(), digest_size=8).digest()
            bucket = int.from_bytes(digest[:4], "little") % 64
            features[192 + bucket] += 0.2 if digest[4] & 1 else -0.2

        for index, pokemon in enumerate(request.get("side", {}).get("pokemon", [])[:6]):
            offset = index * 20
            hp, status = condition(pokemon.get("condition", "0 fnt"))
            features[offset:offset + 4] = [hp, float(pokemon.get("active", False)), float(status == "fnt"), 1]
            for i, name in enumerate(STATUSES):
                features[offset + 4 + i] = float(status == name)
            for i, name in enumerate(("atk", "def", "spa", "spd", "spe")):
                features[offset + 10 + i] = float(pokemon.get("stats", {}).get(name, 0)) / 512
            for key in ("details", "ability", "item", "teraType"):
                if pokemon.get(key):
                    token(f"own{index}:{key}:{pokemon[key]}")
            for move in pokemon.get("moves", []):
                token(f"own{index}:move:{move}")
        for index, active in enumerate(request.get("active", [])[:2]):
            if not active:
                continue
            for move_index, move in enumerate(active.get("moves", [])[:4]):
                features[index * 20 + 15 + move_index] = move.get("pp", 0) / max(move.get("maxpp", 1), 1)
                token(f"active{index}:move{move_index}:{move.get('id', move.get('move', ''))}")
            features[index * 20 + 19] = float(bool(active.get("canTerastallize")))
        for index, (identity, mon) in enumerate(list(view["mons"].items())[:6]):
            offset = 120 + index * 10
            features[offset:offset + 4] = [mon["hp"], float(identity in view["active"].values()), float(mon["status"] == "fnt"), 1]
            for i, status in enumerate(STATUSES):
                features[offset + 4 + i] = float(mon["status"] == status)
            token(f"foe{index}:species:{mon['species']}")
            for reveal in sorted(mon.get("reveals", set())):
                token(f"foe{index}:{reveal}")
        features[180:184] = [min(observation["turn"], 1000) / 100, float(bool(request.get("forceSwitch"))), float(request.get("wait", False)), len(view["mons"]) / 6]
        for key, value in sorted(view["fields"].items()):
            token(f"field:{key}:{value}")
        return features

    @staticmethod
    def _line(view: dict, line: str, own: str) -> None:
        parts = line.split("|")
        if len(parts) < 3:
            return
        event, actor = parts[1:3]
        is_foe = actor.startswith(("p1", "p2")) and not actor.startswith(own)
        slot = actor.split(":", 1)[0]
        identity = actor.split(":", 1)[-1].strip()
        if event in ("switch", "drag", "replace") and is_foe and len(parts) >= 5:
            hp, status = condition(parts[4])
            mon = view["mons"].setdefault(identity, dict(reveals=set()))
            mon.update(species=parts[3], hp=hp, status=status)
            view["active"][slot] = identity
        elif is_foe and identity in view["mons"]:
            mon = view["mons"][identity]
            if event in ("-damage", "-heal", "-sethp") and len(parts) >= 4:
                mon["hp"], mon["status"] = condition(parts[3])
            elif event == "faint":
                mon.update(hp=0, status="fnt")
            elif event == "-status" and len(parts) >= 4:
                mon["status"] = parts[3]
            elif event == "-curestatus":
                mon["status"] = ""
            elif event in ("move", "-item", "-ability", "-terastallize") and len(parts) >= 4:
                mon["reveals"].add(f"{event}:{parts[3]}")
            elif event == "-enditem" and len(parts) >= 4:
                mon["reveals"].add(f"used-item:{parts[3]}")
        if event == "-weather":
            view["fields"]["weather"] = actor
        elif event in ("-fieldstart", "-sidestart"):
            effect = parts[3] if len(parts) > 3 else actor
            view["fields"][f"{event}:{actor}:{effect}"] = effect
        elif event in ("-fieldend", "-sideend"):
            effect = parts[3] if len(parts) > 3 else actor
            view["fields"].pop(f"{event.replace('end', 'start')}:{actor}:{effect}", None)
