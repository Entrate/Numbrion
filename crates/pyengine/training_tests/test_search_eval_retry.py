"""search_eval's protocol-driven players retry choices the engine refuses: an unrevealed Shadow Tag shows only as
maybeTrapped, so a switch can be legal by the request and still be rejected. --opponent search once had no retry
and the ChoiceError ended the whole evaluation."""
import json
import sys
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np

import numbrion as nb
import search_eval
from training import protocol as P

MON = "{name}||Leftovers|{ability}|{moves}||85,85,85,85,85,85|{gender}|||80|,,,,,Normal"


def mon(name, moves, ability, gender="M"):
    return MON.format(name=name, moves=",".join(moves), ability=ability, gender=gender)


FILLER = [mon("Snorlax", ["bodyslam", "protect", "curse", "rest"], "Thick Fat"),
          mon("Blissey", ["hypervoice", "protect", "softboiled", "toxic"], "Healer", "F"),
          mon("Skarmory", ["bravebird", "protect", "roost", "spikes"], "Sturdy"),
          mon("Garchomp", ["earthquake", "protect", "dragonclaw", "swordsdance"], "Rough Skin")]


def side_0_chose(battle) -> bool:
    """Side 0's choice was accepted iff side 1's default choice then plays the turn."""
    turn = battle.turn
    battle.choose(1, "default")
    return battle.turn == turn + 1


def hidden_shadow_tag_battle():
    """Side 0's slot b is trapped by Gothitelle's unrevealed Shadow Tag: its request says only maybeTrapped."""
    ours = [mon("Pikachu", ["fakeout", "protect", "thunderbolt", "volttackle"], "Lightning Rod"),
            mon("Raichu", ["protect", "thunderbolt", "nastyplot", "encore"], "Lightning Rod", "F")] + FILLER
    theirs = [mon("Gothitelle", ["fakeout", "protect", "psychic", "trickroom"], "Shadow Tag", "F"),
              mon("Indeedee", ["protect", "helpinghand", "shadowball", "encore"], "Psychic Surge", "F")] + FILLER
    battle = nb.Battle((1, 2, 3, 4), "]".join(ours), "]".join(theirs), names=("Alice", "Bob"), log=True)
    battle.start()
    assert json.loads(battle.request_json(0))["active"][1].get("maybeTrapped")
    return battle


class RequestPlayer:
    """Plays like a protocol-driven bot: picks from the request mask only. The first pick is a pair the request
    allows but the engine refuses (the hidden trap's switch); later picks are any pair the current request allows."""
    stats = {}

    def __init__(self, battle, side):
        self.refused = [tuple(int(x) for x in p)
                        for p in np.argwhere(P.request_mask(battle.request_json(side))[1] & ~battle.legal_mask(side)[1])]
        self.requests, self.errors, self.budget_ms = [], [], None

    def set_request(self, request):
        self.requests.append(request)

    def rejected(self, error):
        self.errors.append(error)

    def decide(self):
        if not self.errors:
            a0, a1 = self.refused[0]
        else:
            a0, a1 = (int(x) for x in np.argwhere(P.request_mask(self.requests[-1])[1])[0])
        return SimpleNamespace(a0=a0, a1=a1, analysis=dict(source="search", worlds=4))


def test_search_opponent_retries_a_switch_out_of_a_hidden_shadow_tag():
    battle = hidden_shadow_tag_battle()
    player = RequestPlayer(battle, 0)
    assert player.refused and all(P.SWITCH_BASE <= a1 < P.PASS for _, a1 in player.refused)
    opponent = search_eval.SearchOpponent(SimpleNamespace(new_player=lambda name: player), 0, "team", False)
    assert opponent.choose(battle) == 1
    assert "trapped" in player.errors[0]
    assert json.loads(player.requests[-1])["active"][1].get("trapped")  # the rejection revealed the trap
    assert side_0_chose(battle)  # the retry was accepted
    assert opponent.worlds == [4, 4]


def test_no_decision_makes_the_default_choice():
    battle = hidden_shadow_tag_battle()
    player = RequestPlayer(battle, 0)
    player.decide = lambda: None
    assert search_eval.choose_with_retries(player, battle, 0, False, None, []) == 0
    assert side_0_chose(battle)
