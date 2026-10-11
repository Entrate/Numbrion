"""Hand-written protocol scenarios as an observation JSONL (the observations.py format), for the Rust-vs-Python
parity checks of what the fixtures rarely exercise: Illusion, ``swap``, ``-copyboost``, the boost-clearing lines,
``cant`` / ``-fail`` Protect bookkeeping, guard edge cases, odd requests and malformed or non-ASCII lines.

The line lists and request helpers come from ``training_tests/test_selfplay.py``, ``test_effects.py`` and
``test_guards.py`` (copied: those modules import the engine extension). Each scenario is one battle of a few
decision boundaries; ``envs`` battles run side by side as rows ``2 * env + side`` like observations.py. Row ``p1``
gets the scenario's lines and request; row ``p2`` gets the same lines (its tracker sees p1 as the foe) and, when
given, its own request, so the foe matching has partners.

Usage: python scenarios.py OUT.jsonl [--envs 4]
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

# ---- request helpers -------------------------------------------------------------------------------------------


def selfplay_request(names=("Dipplin", "Volbeat"), extra=()):
    """test_selfplay.request."""
    pokemon = []
    for i, name in enumerate(list(names) + list(extra)):
        pokemon.append({"ident": f"p1: {name}", "details": f"{name}, L90", "condition": "100/200", "active": i < 2,
                        "stats": {"atk": 100, "def": 100, "spa": 100, "spd": 100, "spe": 100},
                        "moves": ["protect", "fakeout"], "item": "leftovers", "ability": "intimidate",
                        "teraType": "Water", "terastallized": ""})
    return json.dumps({"active": [{"moves": [{"id": "protect", "pp": 5, "maxpp": 10, "disabled": False},
                                             {"id": "fakeout", "pp": 10, "maxpp": 10, "disabled": True}]}] * 2,
                       "side": {"id": "p1", "pokemon": pokemon}})


SWITCH_IN = ["|switch|p1a: Infernape|Infernape, L85, M|250/250", "|switch|p1b: Dipplin|Dipplin, L90|200/200",
             "|switch|p2a: Inteleon|Inteleon, L78, F|100/100", "|switch|p2b: Gengar|Gengar, L80, M|100/100"]
INFERNAPE = ["fakeout", "closecombat", "protect", "flareblitz"]


def effects_request(tera=True, moves=INFERNAPE, ability="blaze", tera_type="Fire", spe=260):
    """test_effects.request: slot a is Infernape with ``moves``, ``ability`` and ``tera_type``; slot b is Dipplin."""
    team = [("Infernape", moves, ability, tera_type, 85, spe),
            ("Dipplin", ["protect", "dracometeor"], "supersweetsyrup", "Dragon", 90, 100)]
    pokemon = [{"ident": f"p1: {name}", "details": f"{name}, L{level}", "condition": "200/250", "active": True,
                "stats": {"atk": 250, "def": 180, "spa": 250, "spd": 180, "spe": speed}, "moves": known,
                "item": "lifeorb", "ability": skill, "teraType": kind, "terastallized": ""}
               for name, known, skill, kind, level, speed in team]
    active = [{"moves": [{"id": m, "pp": 8, "maxpp": 8, "disabled": False} for m in known],
               **({"canTerastallize": kind} if tera else {})} for _, known, _, kind, _, _ in team]
    return json.dumps({"active": active, "side": {"id": "p1", "pokemon": pokemon}})


def foe_request(names=("Inteleon", "Gengar", "Raichu", "Zoroark"), levels=(78, 80, 85, 80), active=True):
    """A p2 request (the opponent row's own view) so foe tokens have partners to match."""
    pokemon = [{"ident": f"p2: {name}", "details": f"{name}, L{level}", "condition": "80/100", "active": i < 2,
                "stats": {"atk": 150, "def": 120, "spa": 200, "spd": 120, "spe": 180}, "moves": ["protect", "shadowball"],
                "item": "focussash", "ability": "", "baseAbility": "levitate", "teraType": "Ghost", "terastallized": ""}
               for i, (name, level) in enumerate(zip(names, levels))]
    request = {"side": {"id": "p2", "pokemon": pokemon}}
    if active:
        request["active"] = [{"moves": [{"id": "protect", "pp": 3, "maxpp": 16}, {"id": "shadowball", "pp": 0}],
                              "trapped": True}, None]
    return json.dumps(request)


def guard_request(kind):
    """test_guards.test_encoder_clears_guard_on_non_move_requests_and_new_battles."""
    req = json.loads(effects_request())
    if kind == "wait":
        req["wait"] = True
    elif kind == "forced":
        req["forceSwitch"] = [True, False]
    elif kind == "no_active":
        req.pop("active")
    elif kind == "inactive":
        req["side"]["pokemon"][0]["active"] = False
    elif kind == "fainted":
        req["side"]["pokemon"][0]["condition"] = "0 fnt"
    return "" if kind == "empty" else json.dumps(req)


def odd_request():
    """Request quirks: falsy ability (baseAbility used, and the cache key ignores it), a missing stat, no item,
    fewer moves, duplicate request move ids (the last wins), zero / missing maxpp, a null active entry, a fainted
    Pokemon, a non-ASCII name and a status in the condition."""
    def mon(name, details, base, condition="150/300 par", active=False, **extra):
        p = {"ident": f"p1: {name}", "details": details, "condition": condition, "active": active,
             "stats": {"atk": 120, "def": 90, "spa": 80, "spe": 110}, "moves": ["protect", "fakeout", "uturn"],
             "item": "", "ability": "", "baseAbility": base, "teraType": "Stellar", "terastallized": "Stellar"}
        p.update(extra)
        return p
    pokemon = [mon("Café", "Flabébé, L93", "flowerveil", active=True),
               mon("Ditto", "Ditto, L95", "imposter", active=True, condition="0 fnt"),
               mon("Ditto2", "Ditto, L95", "limber"),  # same cache key as Ditto: Python reuses its static part
               mon("Smeargle", "Smeargle, L99, F", "moody", condition="1/1 tox", item="choicescarf", ability="moody",
                   stats={"spe": 200, "atk": 30})]
    active = [{"moves": [{"id": "protect", "pp": 1, "maxpp": 0, "disabled": "hidden"},
                         {"id": "fakeout", "pp": 7, "maxpp": 10},
                         {"id": "protect", "pp": 4, "maxpp": 16, "disabled": False},
                         {"move": "U-turn", "pp": 2}],
               "maybeTrapped": True, "canTerastallize": ""}, None]
    return json.dumps({"active": active, "forceSwitch": [False, False], "side": {"id": "p1", "pokemon": pokemon}})


def zoroark_request():
    """test_guards.test_encoder_never_flags_our_disguised_zoroark_even_with_a_stale_old_record."""
    req = json.loads(effects_request(moves=["fakeout", "firstimpression", "protect", "spikyshield"], ability="illusion"))
    req["side"]["pokemon"][0].update(ident="p1: Zoroark", details="Zoroark, L85")
    return json.dumps(req)


def short_request():
    """test_guards.test_encoder_guards_use_short_request_move_list_and_slot_one."""
    req = json.loads(selfplay_request())
    req["active"][0]["moves"] = [dict(id="fakeout", pp=2, maxpp=10, disabled=False)]
    req["active"][1]["moves"] = [dict(id="firstimpression", pp=2, maxpp=10, disabled=False)]
    return json.dumps(req)


# ---- scenarios: each a list of steps (p1 lines, p1 request, p2 request) -------------------------------------------


def scenarios():
    out = []

    def add(*steps):
        out.append([s if isinstance(s, tuple) else (s, effects_request(), foe_request()) for s in steps])

    # test_selfplay
    public = ["|switch|p1a: Dipplin|Dipplin, L90|200/200", "|switch|p1b: Volbeat|Volbeat, L90|200/200",
              "|switch|p2a: Inteleon|Inteleon, L78, F|100/100", "|switch|p2b: Skuntank|Skuntank, L85, M|100/100",
              "|turn|1", "|move|p2a: Inteleon|Scald|p1a: Dipplin", "|-damage|p1a: Dipplin|100/200",
              "|-boost|p2b: Skuntank|atk|2", "|-unboost|p2a: Inteleon|spe|1",
              "|-damage|p1b: Volbeat|150/200|[from] item: Rocky Helmet|[of] p2b: Skuntank",
              "|-ability|p2a: Inteleon|Torrent", "|-terastallize|p2b: Skuntank|Poison",
              "|-sidestart|p2: Player 2|move: Tailwind", "|-weather|RainDance",
              "|move|p1a: Dipplin|Protect|p1a: Dipplin", "|turn|2"]
    add((public, selfplay_request(), foe_request(("Inteleon", "Skuntank"))))
    phantom = ["|switch|p2a: Inteleon|Inteleon, L78|100/100", "|switch|p2b: Raichu|Raichu, L85|100/100", "|turn|1",
               "|move|p2b: Raichu|Knock Off|p1a: Dipplin", "|-boost|p2b: Raichu|spa|1",
               "|-damage|p2b: Raichu|60/100", "|-damage|p2b: Raichu|48/100|[from] item: Life Orb",
               "|replace|p2b: Zoroark|Zoroark, L80, M", "|-end|p2b: Zoroark|Illusion", "|turn|2"]
    add((phantom, selfplay_request(), foe_request()), (["|move|p2b: Zoroark|Night Daze|p1a: Dipplin", "|turn|3"],
                                                       selfplay_request(), foe_request()))
    seen = ["|switch|p2a: Inteleon|Inteleon, L78|100/100", "|switch|p2b: Raichu|Raichu, L85|100/100", "|turn|1",
            "|move|p2b: Raichu|Thunderbolt|p1a: Dipplin", "|-damage|p2b: Raichu|70/100",
            "|switch|p2b: Garchomp|Garchomp, L80|100/100", "|turn|2"]
    seen2 = ["|switch|p2b: Raichu|Raichu, L85|70/100", "|move|p2b: Raichu|Night Daze|p1a: Dipplin",
             "|move|p2b: Raichu|Thunderbolt|p1a: Dipplin", "|-damage|p2b: Raichu|30/100",
             "|-item|p2b: Raichu|Choice Specs", "|-terastallize|p2b: Raichu|Dark", "|-start|p2b: Raichu|Taunt",
             "|replace|p2b: Zoroark|Zoroark, L80, M, tera:Dark", "|turn|3"]
    add((seen, selfplay_request(), foe_request(("Inteleon", "Garchomp", "Raichu", "Zoroark"))),
        (seen2, selfplay_request(), foe_request(("Inteleon", "Zoroark", "Raichu", "Garchomp"))),
        (["|switch|p2b: Raichu|Raichu, L85|70/100", "|turn|4"], selfplay_request(), ""))
    resets = ["|switch|p2a: Inteleon|Inteleon, L78|100/100", "|switch|p2b: Skuntank|Skuntank, L85|100/100", "|turn|1",
              "|-boost|p2a: Inteleon|spa|2", "|switch|p2a: Garchomp|Garchomp, L80|100/100", "|faint|p2b: Skuntank",
              "|turn|2"]
    add((resets, selfplay_request(), ""), ([], selfplay_request(), ""))

    # test_effects
    turn1 = SWITCH_IN + ["|turn|1", "|move|p1a: Infernape|Protect|p1a: Infernape", "|-singleturn|p1a: Infernape|Protect",
                         "|cant|p1b: Dipplin|flinch", "|cant|p2a: Inteleon|ability: Armor Tail|Encore|[of] p1b: Dipplin",
                         "|turn|2"]
    add(turn1, ["|move|p1a: Infernape|Protect||[still]", "|-fail|p1a: Infernape", "|turn|3"])
    add(SWITCH_IN + ["|turn|1"], ["|move|p1a: Infernape|Close Combat|p2a: Inteleon", "|turn|2"])
    add(SWITCH_IN + ["|-fieldstart|move: Psychic Terrain", "|turn|1"])
    foe_turn = ["|move|p2a: Inteleon|Protect|p2a: Inteleon", "|-singleturn|p2a: Inteleon|Protect"]
    add(SWITCH_IN + ["|turn|1"] + foe_turn + ["|turn|2"], ["|turn|3"])
    add(SWITCH_IN + ["|-ability|p2a: Inteleon|Inner Focus", "|turn|1"])

    # test_guards
    for move in ("fakeout", "firstimpression"):
        moves = [move, "closecombat", "protect", "flareblitz"]
        for action in ("|move|p1a: Infernape|Close Combat|p2a: Inteleon", "|cant|p1a: Infernape|flinch"):
            req = effects_request(moves=moves)
            add((SWITCH_IN + ["|turn|1"], req, foe_request()), ([action, "|turn|2"], req, foe_request()))
    add(SWITCH_IN + ["|turn|1", "|move|p1a: Infernape|Protect|p1a: Infernape", "|-singleturn|p1a: Infernape|Protect",
                     "|turn|2"],
        ["|switch|p1a: Pikachu|Pikachu, L85|200/200", "|turn|3", "|switch|p1a: Infernape|Infernape, L85|200/250",
         "|turn|4"])
    for protect in ("Protect", "Burning Bulwark", "Spiky Shield", "Baneful Bunker"):
        pid = protect.lower().replace(" ", "")
        req = effects_request(moves=["fakeout", pid, "protect", "closecombat"])
        first = SWITCH_IN + ["|turn|1", f"|move|p1a: Infernape|{protect}|p1a: Infernape",
                             f"|-singleturn|p1a: Infernape|{protect}", "|turn|2"]
        add((first, req, ""), ([f"|move|p1a: Infernape|{protect}||[still]", "|-fail|p1a: Infernape", "|turn|3"], req, ""))
        add((first, req, ""), (["|move|p1a: Infernape|Close Combat|p2a: Inteleon", "|turn|3"], req, ""))
        add((SWITCH_IN + ["|turn|1", f"|move|p1a: Infernape|{protect}||[still]", "|-fail|p1a: Infernape", "|turn|2"],
             req, ""))
    add((["|switch|p1a: Dipplin|Dipplin, L90|100/200", "|switch|p1b: Volbeat|Volbeat, L90|100/200", "|turn|1",
          "|move|p1a: Dipplin|Protect|p1a: Dipplin", "|-singleturn|p1a: Dipplin|Protect", "|cant|p1b: Volbeat|flinch",
          "|turn|2"], short_request(), ""))
    protected = SWITCH_IN + ["|turn|1", "|move|p1a: Infernape|Protect|p1a: Infernape",
                             "|-singleturn|p1a: Infernape|Protect", "|turn|2"]
    for kind in ("wait", "forced", "no_active", "inactive", "fainted", "empty"):
        add((protected, effects_request(), foe_request()), ([], guard_request(kind), foe_request(active=False)))
    for prior in (False, True):
        previous = (["|switch|p1a: Zoroark|Zoroark, L85|250/250", "|turn|1", "|move|p1a: Zoroark|Protect|p1a: Zoroark",
                     "|-singleturn|p1a: Zoroark|Protect"] if prior else [])
        add((previous + protected, zoroark_request(), ""))
    # Our disguised Zoroark revealed: the record moves to our own Zoroark (an own key: not in the foe order).
    add((SWITCH_IN + ["|turn|1", "|move|p1a: Infernape|Protect|p1a: Infernape", "|-singleturn|p1a: Infernape|Protect",
                      "|-boost|p1a: Infernape|atk|1", "|replace|p1a: Zoroark|Zoroark, L85, M", "|turn|2"],
         zoroark_request(), foe_request()))

    # Swaps, boost bookkeeping and Protect-like lines the fixtures rarely have.
    add(SWITCH_IN + ["|turn|1", "|-boost|p2a: Inteleon|spa|2", "|-unboost|p2b: Gengar|def|1",
                     "|swap|p2a: Inteleon|1|[from] move: Ally Switch", "|swap|p1b: Dipplin|0", "|-swap|p2a: Gengar|1",
                     "|move|p2b: Inteleon|Ally Switch|p2b: Inteleon", "|turn|2"],
        ["|swap|p2b: Inteleon|0", "|swap|p3a: Nobody|1", "|turn|3"],
        ["|faint|p2a: Inteleon", "|swap|p2b: Gengar|0", "|turn|4"])
    add(["|switch|p1a: Infernape|Infernape, L85, M|250/250", "|switch|p2a: Inteleon|Inteleon, L78, F|100/100",
         "|swap|p2a: Inteleon|1", "|turn|1"],
        ["|replace|p2b: Zoroark|Zoroark, L80", "|swap|p2a: Inteleon|1", "|replace|p2a: Zoroark|Zoroark, L80",
         "|turn|2"])
    add(SWITCH_IN + ["|turn|1", "|-boost|p2a: Inteleon|atk|2", "|-unboost|p2a: Inteleon|spe|3",
                     "|-boost|p2a: Inteleon|evasion|1", "|-copyboost|p2b: Gengar|p2a: Inteleon|[from] move: Psych Up",
                     "|-copyboost|p2a: Newcomer|", "|-copyboost|p2a: Another", "|-clearnegativeboost|p2a: Inteleon",
                     "|-invertboost|p2b: Gengar", "|-setboost|p1a: Infernape|atk|12", "|-setboost|p1a: Infernape|spe",
                     "|-boost|p1b: Dipplin|def|1_0", "|-unboost|p1b: Dipplin|spd|-2", "|turn|2"],
        ["|-clearpositiveboost|p2b: Gengar|p1a: Infernape|move: Spectral Thief", "|-boost|p1a: Infernape|spa|1",
         "|-clearboost|p1b: Dipplin", "|turn|3"],
        ["|-clearallboost", "|-boost|p2a: Inteleon|accuracy|99999999999999999999999", "|turn|4"])
    add(SWITCH_IN + ["|turn|1", "|move|p2a: Inteleon|Detect|p2a: Inteleon", "|-fail|p2a: Inteleon",
                     "|move|p2b: Gengar|Protect||[still]", "|-singleturn|p2b: Gengar|Protect",
                     "|move|p1a: Infernape|Thunder Wave|p2a: Inteleon|[from] ability: Magic Bounce",
                     "|move|p1b: Dipplin|Struggle|p2a: Inteleon", "|cant|p2a: Inteleon|slp", "|cant|p2b: Gengar",
                     "|cant|p9a: Ghost|flinch", "|-fail|p2a: Unknown", "|-singleturn|p1a: Nobody|Protect", "|turn|2"],
        ["|move|p2b: Gengar|Protect|p2b: Gengar", "|-fail|p2b: Gengar|[still]", "|turn|3"],
        ["|move|p1a: Infernape|Protect|p1a: Infernape", "|move|p1a: Infernape|Fake Out|p2a: Inteleon",
         "|-singleturn|p1a: Infernape|Protect", "|turn|4"])

    # Field, side conditions, volatiles, items and abilities from [from] attributions.
    add(SWITCH_IN + ["|-weather|SunnyDay|[from] ability: Drought|[of] p2a: Inteleon", "|-weather|SunnyDay|[upkeep]",
                     "|-fieldstart|move: Grassy Terrain|[from] ability: Grassy Surge|[of] p2b: Gengar",
                     "|-fieldstart|move: Trick Room|[of] p1a: Infernape", "|-fieldstart|move: Gravity",
                     "|-sidestart|p1: Alice|move: Reflect", "|-sidestart|p2: Bob|Spikes", "|-sidestart|p2: Bob|Spikes",
                     "|-sidestart|p2: Bob|move: Toxic Spikes", "|-sidestart|p1: Alice|move: Tailwind",
                     "|-sidestart|p1: Alice|Stealth Rock", "|-sidestart|p1: Alice|move: Wide Guard",
                     "|-start|p2a: Inteleon|perish3", "|-start|p2a: Inteleon|move: Taunt",
                     "|-start|p2a: Inteleon|confusion|[fatigue]", "|-start|p2b: Gengar|Substitute",
                     "|-start|p2b: Gengar|ability: Flash Fire", "|-start|p1a: Infernape|move: Yawn|[of] p2a: Inteleon",
                     "|-item|p2a: Inteleon|Choice Specs", "|-enditem|p2b: Gengar|Focus Sash",
                     "|-ability|p2b: Gengar|Cursed Body|[from] ability: Trace|[of] p1a: Infernape",
                     "|-damage|p1a: Infernape|180/250|[from] item: Rocky Helmet|[of] p2a: Inteleon",
                     "|-heal|p2b: Gengar|100/100|[from] item: Leftovers",
                     "|-heal|p2b: Gengar|100/100|[from] item: Sitrus Berry",
                     "|-immune|p2b: Gengar|[from] ability: Levitate", "|-status|p2a: Inteleon|brn",
                     "|-damage|p2a: Inteleon|88/100 brn|[from] brn", "|turn|1"],
        ["|-start|p2a: Inteleon|perish2", "|-end|p2a: Inteleon|move: Taunt", "|-end|p2b: Gengar|Substitute",
         "|-sideend|p1: Alice|move: Reflect", "|-fieldend|move: Trick Room", "|-weather|DeltaStream",
         "|-curestatus|p2a: Inteleon|brn|[msg]", "|-status|p2b: Gengar|tox", "|-cureteam|p2a: Inteleon",
         "|-terastallize|p1a: Infernape|Fire", "|detailschange|p2a: Inteleon|Inteleon, L78, tera:Water",
         "|-formechange|p2b: Gengar|Gengar-Mega", "|turn|2"],
        ["|-weather|none", "|-fieldend|move: Grassy Terrain", "|-fieldend|move: Gravity", "|-weather|Snow",
         "|-start|p2a: Inteleon|perish0", "|faint|p2a: Inteleon", "|-sethp|p2b: Gengar|50/100", "|turn|3"],
        ["|turn|12"])

    # Malformed and unusual lines: each handler stops at its first bad field and keeps what it already did.
    add(["|switch|p1a: Infernape|Infernape, L85, M|250/250", "|switch|p2a: Café|Flabébé, L90|50/100",
         "|switch|p2b: Gengar|Gengar, L80", "|-boost|p2b: Gengar|atk", "|-boost|p2b: Gengar|spa|x",
         "|-boost|p2b: Gengar|spe| +2 ", "|switch|p2c: Zoroark", "|switch|p2a: Ñandú|Ñandú-Ö, L7, tera:Fairy|33/50 slp",
         "|move|p2a: Ñandú", "|move|p2a: Ñandú|Hyper Voice|p1a: Infernape", "|-status|p2a: Ñandú",
         "|-damage|p2a: Ñandú|abc/100", "|-damage|p2a: Ñandú", "|-damage|p2b: Gengar|50/0",
         "|-damage|p2b: Gengar|1e1/4e1 par extra", "|-heal|p2b: Gengar|50/100/7", "|turn| 3 ",
         "|-sidestart|p2|Spikes", "|-sidestart|p2: Bob", "|-sideend|p2", "|-weather|", "|-start|p2a: Ñandú",
         "|-end|p2a: Ñandú", "|-item|p2a: Ñandú", "|-terastallize|p2a: Ñandú", "|detailschange|p2a: Ñandú",
         "|-ability|p2a: Ñandú", "|replace|p2a", "|swap", "|cant", "|-fail", "|-singleturn", "|faint",
         "|switch|é: Odd|Odd, L50|10/10", "|switch|péa: Odd|Odd, L50|10/10", "|-boost|p2: Café|def|1",
         "|", "||", "x|switch|p2a: Bad|Bad, L1|1/1", "", "|move|", "|-heal|p2b: Gengar|  ", "|turn|1_0"],
        ["|-damage|p2b: Gengar|nan/100", "|-damage|p2a: Café|inf/inf", "|switch|p2b: Ditto|Ditto, L, F, L050|1/1",
         "|-damage|p2a: Ñandú|0", "|turn|-1"],
        ["|switch|p2b: Smeargle|Smeargle,L50,tera:|25/100", "|-damage|p2b: Smeargle|25/100 fnt",
         "|-unboost|p2b: Smeargle|atk|-1", "|turn|+4"])
    # Odd requests on top of ordinary lines.
    add((SWITCH_IN + ["|turn|1"], odd_request(), foe_request()),
        (["|move|p1a: Infernape|Protect|p1a: Infernape", "|-singleturn|p1a: Infernape|Protect", "|turn|2"],
         odd_request(), foe_request(("Gengar", "Inteleon"))))
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("out", type=Path)
    parser.add_argument("--envs", type=int, default=4)
    args = parser.parse_args()
    battles = list(enumerate(scenarios(), start=1))
    source = iter(battles)
    current = [next(source, None) for _ in range(args.envs)]
    position = [0] * args.envs
    n = 0
    with open(args.out, "w", encoding="utf-8") as f:
        while any(c is not None for c in current):
            batch = []
            for e in range(args.envs):
                if current[e] is None:
                    batch += [{"battle_id": -1, "request": "", "log": []}] * 2
                    continue
                battle_id, steps = current[e]
                lines, request1, request2 = steps[position[e]]
                batch.append({"battle_id": battle_id, "request": request1, "log": lines})
                batch.append({"battle_id": battle_id, "request": request2, "log": lines})
            f.write(json.dumps({"observations": batch}) + "\n")
            n += 1
            for e in range(args.envs):
                if current[e] is None:
                    continue
                position[e] += 1
                if position[e] >= len(current[e][1]):
                    current[e], position[e] = next(source, None), 0
    print(f"{len(battles)} scenarios, {n} boundaries x {2 * args.envs} rows -> {args.out}")


if __name__ == "__main__":
    main()
