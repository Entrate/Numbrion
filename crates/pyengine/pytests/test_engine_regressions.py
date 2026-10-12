"""Battles that once panicked inside the engine, replayed from BatchEnv crash reports (`BatchEnv.crashes`)."""
import pytest

import numbrion as nb


def replay(seed, teams, actions):
    b = nb.Battle(tuple(seed), teams[0], teams[1], log=True)
    b.start()
    lines = list(b.drain_log())
    for row in actions:
        for side in (0, 1):
            if row[2 * side] >= 0:
                b.choose_action(side, row[2 * side], row[2 * side + 1])
        lines += b.drain_log()
    return b, lines


# Calyrex (Unnerve) switches in while Moltres (Berserk, Sitrus Berry) is low. Unnerve's onFoeTryEatItem returns
# true before its unnerved state is set, so Berserk's onTryEatItem(item) receives true, not the item. Showdown reads
# true.id as undefined and continues; the engine panicked with "callback requires effect argument".
BERSERK_AFTER_UNNERVE = dict(
    seed=[393, 53885, 24710, 32085],
    teams=['Pachirisu||SitrusBerry|VoltAbsorb|thunderbolt,superfang,followme,nuzzle||85,85,85,85,85,85|F|||93|,,,,,Water]Seviper||LifeOrb|ShedSkin|flamethrower,knockoff,gunkshot,protect||85,85,85,85,85,85|M|||96|,,,,,Poison]Pyroar||LifeOrb|Unnerve|fireblast,willowisp,hypervoice,protect||85,,85,85,85,85|F|,0,,,,||83|,,,,,Water]Slither Wing||LifeOrb|Protosynthesis|closecombat,firstimpression,flareblitz,wildcharge||85,85,85,85,85,85|N|||83|,,,,,Fighting]Calyrex||SitrusBerry|Unnerve|helpinghand,psychic,pollenpuff,gigadrain||85,,85,85,85,85|N|,0,,,,||94|,,,,,Steel]Leafeon||LifeOrb|Chlorophyll|synthesis,leafblade,swordsdance,doubleedge||85,85,85,85,85,85|M|||91|,,,,,Normal', 'Jirachi||ChoiceScarf|SereneGrace|thunderbolt,ironhead,uturn,psychic||85,85,85,85,85,85|N|||80|,,,,,Steel]Moltres|MoltresGalar|SitrusBerry|Berserk|fierywrath,hurricane,tailwind,nastyplot||85,,85,85,85,85|N|,0,,,,||74|,,,,,Dark]Toxapex||Leftovers|Regenerator|recover,infestation,toxic,banefulbunker||85,,85,85,85,85|M|,0,,,,||92|,,,,,Steel]Furret||LifeOrb|Frisk|doubleedge,tidyup,protect,knockoff||85,85,85,85,85,85|M|||99|,,,,,Normal]Meganium||SitrusBerry|Overgrow|encore,knockoff,healpulse,energyball||85,85,85,85,85,85|F|||91|,,,,,Water]Wigglytuff||SitrusBerry|Competitive|encore,fireblast,thunderwave,dazzlinggleam||85,,85,85,85,85|M|,0,,,,||90|,,,,,Fire'],
    actions=[[8, 42, 18, 16], [30, 24, 16, 35], [18, 34, 42, 24], [38, 34, 42, 24], [44, 34, 36, 24]],
)


def test_berserk_try_eat_item_after_unnerve_relay():
    b, lines = replay(**BERSERK_AFTER_UNNERVE)
    assert not b.ended
    assert "|-ability|p1a: Calyrex|Unnerve" in lines
    assert lines[-1] == "|turn|6"  # the panicking turn completes (same log as Showdown 7332b60)


# Pawmot uses Revival Blessing with no living Pokemon in reserve. Showdown (7332b60, and upstream as of 2026-10-12)
# accepts `pass` for the revive slot, then throws "Choices are done immediately after a request" at the turn's next
# makeRequest; the engine reproduced it as a panic. The masks leave pass out of a Revival Blessing slot, so the side
# revives someone and the battle goes on (the same log as Showdown to the end).
REVIVAL_BLESSING_PASS = dict(
    seed=[8154, 59923, 30018, 5705],
    teams=['Drifblim||SitrusBerry|Unburden|strengthsap,tailwind,shadowball,willowisp||85,,85,85,85,85|F|,0,,,,||85|,,,,,Ground]Flamigo||ChoiceBand|Scrappy|throatchop,closecombat,bravebird,uturn||85,85,85,85,85,85|M|||85|,,,,,Fighting]Venusaur||LifeOrb|Overgrow|protect,sludgebomb,knockoff,leafstorm||85,85,85,85,85,85|M|||87|,,,,,Dark]Swampert||AssaultVest|Torrent|flipturn,highhorsepower,muddywater,icywind||85,85,85,85,85,85|F|||82|,,,,,Steel]Pawmot||LifeOrb|VoltAbsorb|doubleshock,revivalblessing,closecombat,protect||85,85,85,85,85,85|M|||80|,,,,,Electric]Dudunsparce||SitrusBerry|Rattled|tailwind,hyperdrill,protect,earthpower||85,85,85,85,85,85|F|||86|,,,,,Ground', 'Rabsca||SitrusBerry|Synchronize|trickroom,psychic,strugglebug,revivalblessing||85,,85,85,85,|M|,0,,,,0||87|,,,,,Steel]Frosmoth||HeavyDutyBoots|IceScales|icebeam,protect,terablast,quiverdance||85,,85,85,85,85|F|,0,,,,||86|,,,,,Ground]Tauros|TaurosPaldeaAqua|ChoiceBand|Intimidate|zenheadbutt,wavecrash,closecombat,aquajet||85,85,85,85,85,85|M|||80|,,,,,Water]Arcanine|ArcanineHisui|ChoiceBand|Intimidate|extremespeed,stoneedge,rockslide,flareblitz||85,85,85,85,85,85|F|||79|,,,,,Rock]Empoleon||SitrusBerry|Competitive|flashcannon,hydropump,yawn,icywind||85,,85,85,85,85|F|,0,,,,||83|,,,,,Flying]Landorus|LandorusTherian|SitrusBerry|Intimidate|stompingtantrum,taunt,stoneedge,uturn||85,85,85,85,85,85|M|||77|,,,,,Steel'],
    actions=[[14, 28, 25, 14], [14, 22, 10, 14], [15, 22, 10, 34], [43, 46, -1, -1], [34, 26, 18, 34], [46, 42, -1, -1], [10, 36, 34, 14], [24, 38, 16, 8], [46, 45, -1, -1], [0, 24, 10, 43], [0, 12, 4, 6], [44, 46, -1, -1], [0, 12, 4, 8], [34, 4, 10, 8], [28, 16, 10, 8], [46, 44, 46, 43], [14, 2, 24, 14]],
)


def test_revival_blessing_slot_offers_no_pass():
    b, _ = replay(**REVIVAL_BLESSING_PASS)
    assert b.request_kind(0) == "switch"
    m0, joint = b.legal_mask(0)
    assert not m0[nb.PASS]
    revive = [int(a) for a in m0.nonzero()[0]]
    assert revive and all(nb.describe_action(a).startswith("switch") for a in revive)
    with pytest.raises(nb.ChoiceError):
        b.choose_action(0, nb.PASS, nb.PASS)
    b.choose_action(0, revive[0], int(joint[revive[0]].nonzero()[0][0]))
    lines = b.drain_log()
    assert any(line.startswith("|-heal|") and "revivalblessing" in line.replace(" ", "").lower() for line in lines)
    assert not b.ended


def test_batch_env_avoids_the_revival_blessing_pass():
    teams = REVIVAL_BLESSING_PASS["teams"]
    env = nb.BatchEnv(16, teams=teams * 4, seed=3, threads=2)
    stats = env.run_random(30, switch_prob=0.5)
    assert stats["crashed"] == 0 and not env.crashes()
