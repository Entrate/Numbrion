"""The scenario suite (format: core.py). Every scenario was checked with the engine (``run.py --check`` and
``--probe``); the rationale states what the probes showed.

Opponent sets are taken verbatim from the random-doubles set pool (data/teams/pool-s1-200k.txt.gz) so the world
builder can reproduce them. Our own sets may be anything the engine accepts (the player knows its own team).

Endgames use two-Pokemon teams thinned on turn 1: the opponent's Choice Scarf Annihilape Final Gambits our level-1
Pikachu, so both faint and a 1v1 (or 2v1) remains with real fainted teammates (one-Pokemon teams are not valid).
"""
from __future__ import annotations

from tools.scenarios.core import Expect, Scenario, Target


def team(*mons: str) -> str:
    return "]".join(mons)


# ---- opponent sets (pool) ------------------------------------------------------------------------------------------
AMOONGUSS = "Amoonguss||RockyHelmet|Regenerator|spore,pollenpuff,ragepowder,clearsmog||85,,85,85,85,85|M|,0,,,,||86|,,,,,Water"
INCINEROAR = "Incineroar||HeavyDutyBoots|Intimidate|fakeout,partingshot,flareblitz,knockoff||85,85,85,85,85,85|M|||78|,,,,,Water"
VIVILLON = "Vivillon|VivillonPolar|FocusSash|CompoundEyes|protect,tailwind,sleeppowder,hurricane||85,,85,85,85,85|M|,0,,,,||88|,,,,,Flying"
QUAGSIRE = "Quagsire||SitrusBerry|WaterAbsorb|liquidation,highhorsepower,yawn,icywind||85,85,85,85,85,85|M|||91|,,,,,Poison"
GASTRODON = "Gastrodon||Leftovers|StormDrain|recover,yawn,earthpower,icebeam||85,,85,85,85,85|F|,0,,,,||82|,,,,,Fire"
GARCHOMP = "Garchomp||LoadedDice|RoughSkin|protect,scaleshot,swordsdance,earthquake||85,85,85,85,85,85|M|||77|,,,,,Dragon"
CORVIKNIGHT = "Corviknight||SitrusBerry|MirrorArmor|tailwind,roost,ironhead,bravebird||85,85,85,85,85,85|F|||80|,,,,,Dragon"
KINGAMBIT = "Kingambit||SitrusBerry|Defiant|ironhead,suckerpunch,swordsdance,protect||85,85,85,85,85,85|M|||77|,,,,,Flying"
TYRANITAR = "Tyranitar||AssaultVest|SandStream|highhorsepower,icywind,knockoff,stoneedge||85,85,85,85,85,85|F|||81|,,,,,Flying"
GREAT_TUSK = "Great Tusk||SitrusBerry|Protosynthesis|closecombat,protect,rapidspin,headlongrush||85,85,85,85,85,85|N|||81|,,,,,Ground"
BLISSEY = "Blissey||SitrusBerry|Healer|helpinghand,hypervoice,softboiled,thunderwave||85,,85,85,85,85|F|,0,,,,||96|,,,,,Poison"
DRAGAPULT = "Dragapult||ChoiceBand|ClearBody|phantomforce,uturn,dragondarts,dragonclaw||85,85,85,85,85,85|M||S|79|,,,,,Dragon"
ANNIHILAPE_GAMBIT = "Annihilape||ChoiceScarf|Defiant|uturn,ragefist,finalgambit,closecombat||85,85,85,85,85,85|F|||77|,,,,,Fighting"
THEIR_BENCH = [GARCHOMP, CORVIKNIGHT, TYRANITAR, BLISSEY]

# ---- our sets ------------------------------------------------------------------------------------------------------
VOLCARONA = "Volcarona||HeavyDutyBoots|FlameBody|quiverdance,fierydance,gigadrain,protect||85,,85,85,85,85|M|,0,,,,||78|,,,,,Grass"
VOLCARONA_OVERCOAT = VOLCARONA.replace("FlameBody", "Overcoat")
TALONFLAME = "Talonflame||SitrusBerry|GaleWings|bravebird,tailwind,overheat,protect||85,85,85,85,85,85|F|||82|,,,,,Flying"
PINCURCHIN = "Pincurchin||SitrusBerry|ElectricSurge|thunderbolt,electroweb,recover,scald||85,,85,85,85,85|F|,0,,,,||97|,,,,,Water"
OUR_GARCHOMP = "Garchomp||LoadedDice|RoughSkin|scaleshot,earthquake,swordsdance,protect||85,85,85,85,85,85|M|||77|,,,,,Fire"
LUCARIO = "Lucario||LifeOrb|InnerFocus|closecombat,extremespeed,flashcannon,protect||85,85,85,85,85,85|M|||86|,,,,,Normal"
OUR_KINGAMBIT = "Kingambit||Leftovers|Defiant|kowtowcleave,ironhead,suckerpunch,protect||85,85,85,85,85,85|F|||77|,,,,,Dark"
OUR_CORVIKNIGHT = "Corviknight||Leftovers|Pressure|bodypress,bravebird,roost,ironhead||85,85,85,85,85,85|F|||80|,,,,,Fighting"
OUR_SCIZOR = "Scizor||Leftovers|Technician|bulletpunch,closecombat,uturn,protect||85,85,85,85,85,85|M|||80|,,,,,Steel"
PELIPPER = "Pelipper||Leftovers|Drizzle|wideguard,hurricane,weatherball,protect||85,,85,85,85,85|M|,0,,,,||83|,,,,,Water"
ARCANINE = "Arcanine||SitrusBerry|Intimidate|flareblitz,crunch,protect,willowisp||85,85,85,85,85,85|M|||84|,,,,,Normal"
OUR_RILLABOOM = "Rillaboom||Leftovers|GrassySurge|woodhammer,grassyglide,uturn,protect||85,85,85,85,85,85|M|||81|,,,,,Grass"
GARCHOMP_TERA = "Garchomp||LifeOrb|RoughSkin|scaleshot,earthquake,terablast,protect||85,85,85,85,85,85|M|||77|,,,,,Electric"
FODDER = "Pikachu||Leftovers|Static|helpinghand,protect||85,85,85,85,85,85|M|||1|,,,,,Electric"
OUR_BENCH = [
    "Dragonite||LoadedDice|InnerFocus|tailwind,fireblast,scaleshot,protect||85,85,85,85,85,85|F|||82|,,,,,Fire",
    "Azumarill||ChoiceBand|HugePower|aquajet,liquidation,knockoff,playrough||85,85,85,85,85,85|M|||83|,,,,,Water",
    "Lucario||LifeOrb|InnerFocus|closecombat,extremespeed,flashcannon,protect||85,85,85,85,85,85|M|||86|,,,,,Normal",
    "Metagross||WeaknessPolicy|ClearBody|heavyslam,protect,psychicfangs,agility||85,85,85,85,85,85|N|||80|,,,,,Dragon",
]
OUR_BENCH_GRASS = [OUR_RILLABOOM] + OUR_BENCH[:3]

SCENARIOS = [
    # ---- sleep -----------------------------------------------------------------------------------------------------
    Scenario(
        name="sleep-spore-setup",
        family="sleep",
        teams=(team(VOLCARONA, TALONFLAME, *OUR_BENCH), team(AMOONGUSS, INCINEROAR, *THEIR_BENCH)),
        seed=(11, 22, 33, 44),
        script=[("move 4, move 1 +1", "move 1 +1, move 4 +2")],
        target=Target(turn=2, mons={(1, "amoonguss"): ["active", "hp<60"], (0, "volcarona"): ["active", "ok"]},
                      log=["|move|p2a: Amoonguss|Spore|p1a: Volcarona"]),
        expect=Expect(reject={0: ["move:quiverdance*"]}),
        rationale="Amoonguss (55%) revealed Spore into Volcarona's Protect. Volcarona outspeeds it and Fiery Dance "
                  "KOs it before it moves (8/8 probes). Quiver Dance lets Amoonguss put Volcarona to sleep with the "
                  "boosts unused.",
    ),
    Scenario(
        name="sleep-spore-setup-overcoat",
        family="sleep",
        teams=(team(VOLCARONA_OVERCOAT, TALONFLAME, *OUR_BENCH), team(AMOONGUSS, INCINEROAR, *THEIR_BENCH)),
        seed=(11, 22, 33, 44),
        script=[("move 4, move 1 +1", "move 1 +1, move 4 +2")],
        target=Target(turn=2, mons={(1, "amoonguss"): ["active"], (0, "volcarona"): ["active", "ok"]},
                      log=["|move|p2a: Amoonguss|Spore|p1a: Volcarona"]),
        rationale="Informational, the contrast to sleep-spore-setup: Volcarona holds Safety Goggles, so Spore cannot "
                  "touch it. Quiver Dance should be chosen more often here than there.",
    ),
    Scenario(
        name="sleep-electric-terrain",
        family="sleep",
        teams=(team(VOLCARONA, PINCURCHIN, *OUR_BENCH), team(AMOONGUSS, INCINEROAR, *THEIR_BENCH)),
        seed=(11, 22, 33, 44),
        script=[("move 4, move 1 +1", "move 1 +1, move 4 +2")],
        target=Target(turn=2, mons={(0, "volcarona"): ["active", "ok"]},
                      log=["|-fieldstart|move: Electric Terrain", "|move|p2a: Amoonguss|Spore|p1a: Volcarona"]),
        rationale="Informational: Pincurchin's Electric Terrain (turn 2 of 5) keeps grounded Pokemon from falling "
                  "asleep, so Spore cannot stop Volcarona's Quiver Dance this turn.",
    ),
    Scenario(
        name="sleep-powder-ko-first",
        family="sleep",
        teams=(team(OUR_GARCHOMP, LUCARIO, *OUR_BENCH[:2], OUR_BENCH[3]), team(VIVILLON, INCINEROAR, *THEIR_BENCH)),
        seed=(5, 6, 7, 8),
        script=[("move 4, move 3 +1", "move 3 +1, move 3 +1")],
        target=Target(turn=2, mons={(1, "vivillonpolar"): ["active", "hp<50"], (0, "garchomp"): ["active", "ok"]},
                      log=["|move|p2a: Vivillon|Sleep Powder|p1a: Garchomp"]),
        expect=Expect(any_slot=["move:extremespeed@+1*"]),
        rationale="Vivillon revealed Sleep Powder (Compound Eyes: 97.5% accurate) into Garchomp's Protect and is low "
                  "after Flash Cannon. Lucario's Extreme Speed (+2 priority) KOs it before it can move; anything "
                  "else gives it another Sleep Powder.",
    ),
    Scenario(
        name="sleep-switch-to-grass",
        family="sleep",
        teams=(team(OUR_GARCHOMP, OUR_KINGAMBIT, *OUR_BENCH_GRASS), team(AMOONGUSS, INCINEROAR, *THEIR_BENCH)),
        seed=(3, 1, 4, 1),
        script=[("move 4, move 4", "move 1 +1, move 4 +2")],
        target=Target(turn=2, mons={(0, "garchomp"): ["active", "ok"], (1, "amoonguss"): ["active"]},
                      log=["|move|p2a: Amoonguss|Spore|p1a: Garchomp"]),
        rationale="Informational: Amoonguss (full HP, Rocky Helmet, Regenerator) revealed Spore into Garchomp's "
                  "Protect; Garchomp cannot KO it. Switching Garchomp to the Rillaboom on the bench (Grass: immune "
                  "to Spore) is the textbook answer; report how often each config switches.",
    ),
    # ---- yawn -------------------------------------------------------------------------------------------------------
    Scenario(
        name="yawn-drowsy-no-protect",
        family="yawn",
        teams=(team(OUR_GARCHOMP, OUR_KINGAMBIT), team(QUAGSIRE, CORVIKNIGHT)),
        seed=(9, 9, 9, 9),
        script=[("move 1 +2, move 2 +1", "move 3 +1, move 1")],
        target=Target(turn=2, mons={(0, "garchomp"): ["active", "ok"], (0, "kingambit"): ["active"]},
                      log=["|-start|p1a: Garchomp|move: Yawn"]),
        expect=Expect(reject={0: ["move:protect*"]}),
        rationale="Garchomp is drowsy from Yawn and falls asleep at the end of this turn whatever it does; there is "
                  "no bench to switch to. Protect cannot stop the sleep and wastes Garchomp's last turn awake.",
    ),
    Scenario(
        name="yawn-drowsy-switch",
        family="yawn",
        teams=(team(OUR_GARCHOMP, OUR_KINGAMBIT, *OUR_BENCH), team(GASTRODON, CORVIKNIGHT, *THEIR_BENCH[:1],
                                                                    *THEIR_BENCH[2:])),
        seed=(9, 9, 9, 9),
        script=[("move 1 +2, move 2 +1", "move 2 +1, move 1")],
        target=Target(turn=2, mons={(0, "garchomp"): ["active", "ok"]}, log=["|-start|p1a: Garchomp|move: Yawn"]),
        rationale="Informational: Garchomp is drowsy (Gastrodon's Yawn) and four healthy teammates are on the bench. "
                  "Switching out clears the drowsiness; staying in gives one attack, then sleep. Report how often "
                  "each config switches Garchomp out.",
    ),
    # ---- endgames ---------------------------------------------------------------------------------------------------
    Scenario(
        name="endgame-roost-or-finish",
        family="endgame",
        teams=(team(OUR_CORVIKNIGHT, FODDER), team(TYRANITAR, ANNIHILAPE_GAMBIT)),
        seed=(1, 2, 3, 4),
        script=[("move 2 +1, move 1 -1", "move 4 +1, move 3 +2"),
                ("move 2 +1", "move 4 +1")],
        target=Target(turn=3, mons={(0, "corviknight"): ["active"], (1, "tyranitar"): ["active"]}),
        expect=Expect(accept={0: ["move:bodypress*"]}),
        rationale="1v1, last Pokemon each: Corviknight 55% (faster) against Tyranitar 59%. Body Press (4x) KOs it "
                  "now (10/10 probes); Iron Head leaves it at ~18%, Brave Bird is resisted. Roost is safe but only "
                  "gives Tyranitar more Stone Edges (and crits) and delays the win.",
    ),
    Scenario(
        name="endgame-2v1-attack-both",
        family="endgame",
        teams=(team(OUR_SCIZOR, FODDER, OUR_KINGAMBIT), team(GARCHOMP, ANNIHILAPE_GAMBIT)),
        seed=(2, 4, 6, 8),
        script=[("move 2 +1, move 1 -1", "move 3, move 3 +2"),
                ("switch 3", None)],
        target=Target(turn=2, mons={(0, "scizor"): ["active"], (0, "kingambit"): ["active"],
                                    (1, "garchomp"): ["active", "hp<50"]},
                      log=["|-boost|p2a: Garchomp|atk|2"]),
        expect=Expect(accept={0: ["move:bulletpunch@+1*", "move:closecombat@+1*"],
                              1: ["move:suckerpunch@+1*", "move:kowtowcleave@+1+tera"]}),
        rationale="2v1: Scizor and Kingambit against a +2 Garchomp (46%) that outspeeds both; its Earthquake KOs "
                  "Kingambit (Steel) and badly hurts Scizor. Both must attack it, and Kingambit must either hit "
                  "first (Sucker Punch) or Tera Dark to survive the Earthquake: Bullet Punch + Sucker Punch and "
                  "Bullet Punch + Tera Kowtow Cleave win 8/8 probes (Close Combat + Tera Kowtow Cleave 7/8); without "
                  "Tera, Bullet Punch + Kowtow Cleave wins 4/8 and Close Combat + Kowtow Cleave 1/8; double Protect "
                  "only delays and lets it boost again.",
    ),
    Scenario(
        name="endgame-last-mon-finish",
        family="endgame",
        teams=(team(OUR_GARCHOMP, FODDER), team(GREAT_TUSK, ANNIHILAPE_GAMBIT)),
        seed=(7, 7, 7, 7),
        script=[("move 2, move 1 -1", "move 4 +1, move 3 +2"),
                ("move 3", "move 2")],
        target=Target(turn=3, mons={(0, "garchomp"): ["active", "hp<60"], (1, "greattusk"): ["active", "hp<80"]},
                      log=["|-boost|p1a: Garchomp|atk|2", "|-singleturn|p2a: Great Tusk|Protect"]),
        expect=Expect(accept={0: ["move:scaleshot*", "move:earthquake*"]}),
        rationale="Our last Pokemon, a +2 Garchomp (51%, faster), against our last foe, Great Tusk (72%, -1 Def), "
                  "which just used Protect. +2 Earthquake KOs it now (10/10 probes; Scale Shot 8/10); Protect only "
                  "delays, and Tusk's Headlong Rush (~49%) nearly KOs Garchomp.",
    ),
    Scenario(
        name="endgame-tera-only-out",
        family="endgame",
        teams=(team(GARCHOMP_TERA, FODDER), team(CORVIKNIGHT, ANNIHILAPE_GAMBIT)),
        seed=(3, 3, 3, 3),
        script=[("move 1 +1, move 1 -1", "move 4 +1, move 3 +2")],
        target=Target(turn=2, mons={(0, "garchomp"): ["active"], (1, "corviknight"): ["active"]}),
        expect=Expect(accept={0: ["move:terablast*+tera"]}),
        rationale="1v1: Garchomp (61%, faster) against Corviknight (74%); Scale Shot is resisted and Earthquake "
                  "cannot hit it, and Brave Bird does ~39% to Garchomp. Tera Electric Tera Blast (2x, STAB) KOs it "
                  "now half the time and leaves it in range of a finishing hit otherwise; without Tera Garchomp "
                  "loses the race.",
    ),
    Scenario(
        name="endgame-priority-vs-boosted",
        family="endgame",
        teams=(team(OUR_SCIZOR, FODDER), team(GARCHOMP, ANNIHILAPE_GAMBIT)),
        seed=(4, 4, 4, 4),
        script=[("move 2 +1, move 1 -1", "move 3, move 3 +2"),
                ("move 2 +1", "move 3")],
        target=Target(turn=3, mons={(0, "scizor"): ["active"], (1, "garchomp"): ["active", "hp<25"]},
                      log=["|-boost|p2a: Garchomp|atk|2"]),
        expect=Expect(accept={0: ["move:bulletpunch*"]}),
        rationale="1v1 against a +4 Garchomp (17%) that outspeeds Scizor and KOs it with Scale Shot. Protect only "
                  "delays the KO; Bullet Punch (priority) KOs Garchomp first (8/8 probes; Close Combat loses 3/4).",
    ),
    Scenario(
        name="endgame-wide-guard-pointless",
        family="endgame",
        teams=(team(PELIPPER, FODDER, LUCARIO), team(KINGAMBIT, ANNIHILAPE_GAMBIT)),
        seed=(5, 5, 5, 5),
        script=[("move 2 +1, move 1 -1", "move 1 +1, move 3 +2"),
                ("switch 3", None)],
        target=Target(turn=2, mons={(0, "pelipper"): ["active"], (0, "lucario"): ["active"],
                                    (1, "kingambit"): ["active"]}),
        expect=Expect(reject={0: ["move:wideguard*"]}),
        rationale="2v1 against Kingambit, whose sets (pool) have no spread move: Wide Guard can never block "
                  "anything. Pelipper should attack (or at least not waste the turn).",
    ),
    # ---- controls ---------------------------------------------------------------------------------------------------
    Scenario(
        name="control-protect-burn-out",
        family="control",
        teams=(team(ARCANINE, FODDER), team(DRAGAPULT, ANNIHILAPE_GAMBIT)),
        seed=(59415, 3721, 48210, 32782),
        script=[("move 4 +1, move 1 -1", "move 4 +1, move 3 +2"),
                ("move 2 +1", "move 4 +1"),
                ("move 1 +1", "move 4 +1")],
        target=Target(turn=4, mons={(0, "arcanine"): ["active", "hp<=20"],
                                    (1, "dragapult"): ["active", "brn", "hp<=6"]}),
        expect=Expect(accept={0: ["move:protect*"]}),
        rationale="Stalling is right: Dragapult (burned, 9/269 HP, under the burn's 16) outspeeds Arcanine (17%) "
                  "and KOs it with Dragon Claw. Protect blocks it and the burn KOs Dragapult at the end of the turn "
                  "(5/5 probes win); Crunch or Flare Blitz lose 5/5.",
    ),
    Scenario(
        name="control-sleep-clause",
        family="control",
        teams=(team(VOLCARONA, TALONFLAME, *OUR_BENCH), team(AMOONGUSS, INCINEROAR, *THEIR_BENCH)),
        seed=(11, 22, 33, 44),
        script=[("move 4, move 2", "move 1 +2, move 4 +2")],
        target=Target(turn=2, mons={(0, "talonflame"): ["active", "slp"], (0, "volcarona"): ["active", "ok"]}),
        rationale="Informational, the contrast to sleep-spore-setup: Talonflame is already asleep from Spore, so the "
                  "Sleep Clause makes a second Spore fail. Volcarona can Quiver Dance without the sleep threat.",
    ),
    Scenario(
        name="control-turn1-fake-out",
        family="control",
        teams=(team(VOLCARONA, TALONFLAME, *OUR_BENCH), team(INCINEROAR, GARCHOMP, AMOONGUSS, CORVIKNIGHT,
                                                               TYRANITAR, BLISSEY)),
        seed=(1, 1, 2, 3),
        script=[],
        target=Target(turn=1),
        rationale="Informational: turn 1 against Incineroar (Fake Out in every pool set) and Garchomp. How often each "
                  "config protects, Tailwinds or attacks.",
    ),
]
