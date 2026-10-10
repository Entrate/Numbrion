//! One case per line shape of docs/showdown/05-log-shapes.txt (all 168), built the way an engine
//! call site builds it. The expected lines are the catalog's real examples with the standard
//! teams' names substituted (`replay.rs` independently replays real lines for every shape).
//!
//! Field: p1a Wugtrio, p1b Dondozo, p2a Klefki, p2b Reshiram. Bench: p1 Gholdengo (2),
//! Deoxys-Defense (3), Zoroark (4), Porygon2 (5); p2 Basculegion-F (8), Arceus-Bug (9),
//! Ogerpon-Wellspring (10), Greninja-Bond (11).
use super::*;
use LogArg::{EffectFullName as Full, Empty, Mon, Number, Text};

const W: MonId = MonId(0);
const D: MonId = MonId(1);
const K: MonId = MonId(6);
const R: MonId = MonId(7);
const GHOLDENGO: MonId = MonId(2);

fn eff(name: &str) -> LogArg<'static> {
    // The move/ability/item namespaces do not collide on these names.
    for kind in [EffectKind::Move, EffectKind::Ability, EffectKind::Item] {
        if let Some(id) = dex::lookup(kind, name) {
            return LogArg::Effect(EffectRef::Dex(id));
        }
    }
    panic!("unknown effect {name}")
}

fn full(kind: EffectKind, name: &str) -> LogArg<'static> {
    Full(fx(kind, name))
}

fn from(kind: EffectKind, name: &str) -> LogTag<'static> {
    LogTag::From(fx(kind, name))
}

fn is(
    b: &mut Battle<TextLog>,
    want: &str,
    cmd: &'static str,
    args: &[LogArg<'_>],
    tags: &[LogTag<'_>],
) {
    let got = add(b, cmd, args, tags);
    assert_eq!(got, [want], "{cmd}");
}

/// A split entry: `[split, secret, public]`.
fn split(
    b: &mut Battle<TextLog>,
    want: [&str; 3],
    cmd: &'static str,
    args: &[LogArg<'_>],
    tags: &[LogTag<'_>],
) {
    let side = args
        .iter()
        .find_map(|a| {
            if let Mon(m) = a {
                Some(m.side().0)
            } else {
                None
            }
        })
        .unwrap();
    let got = add_split(b, cmd, args, tags, side, false);
    assert_eq!(got, want, "{cmd}");
}

fn set_hp(b: &mut Battle<TextLog>, m: MonId, hp: u16, max: u16, status: &str) {
    let p = &mut b.state.pokemon[m.0 as usize];
    p.hp = hp;
    p.max_hp = max;
    p.status = status_of(status);
}

#[test]
fn constructor_and_turn_structure_lines() {
    let mut b = standard_active();
    is(&mut b, "|", "", &[], &[]);
    // battle.add('t:', Date.now()): fixtures hold the normalized bare `|t:|`.
    is(&mut b, "|t:|", "t:", &[Empty], &[]);
    is(&mut b, "|t:|", "t:", &[], &[]);
    is(
        &mut b,
        "|gametype|doubles",
        "gametype",
        &[Text("doubles")],
        &[],
    );
    is(
        &mut b,
        "|player|p1|Alice||",
        "player",
        &[
            LogArg::SideId(SideId(0)),
            LogArg::PlayerName(SideId(0)),
            Empty,
            Empty,
        ],
        &[],
    );
    is(
        &mut b,
        "|player|p2|Bob||",
        "player",
        &[
            LogArg::SideId(SideId(1)),
            LogArg::PlayerName(SideId(1)),
            Empty,
            Empty,
        ],
        &[],
    );
    is(&mut b, "|gen|9", "gen", &[Number(9)], &[]);
    is(
        &mut b,
        "|tier|[Gen 9] Random Doubles Battle",
        "tier",
        &[Text("[Gen 9] Random Doubles Battle")],
        &[],
    );
    for rule in [
        "Species Clause: Limit one of each Pokémon",
        "HP Percentage Mod: HP is shown in percentages",
        "Illusion Level Mod: Illusion disguises the Pokémon's true level",
        "Sleep Clause Mod: Limit one foe put to sleep",
    ] {
        is(&mut b, &format!("|rule|{rule}"), "rule", &[Text(rule)], &[]);
    }
    is(
        &mut b,
        "|teamsize|p1|6",
        "teamsize",
        &[LogArg::SideId(SideId(0)), Number(6)],
        &[],
    );
    is(
        &mut b,
        "|teamsize|p2|6",
        "teamsize",
        &[LogArg::SideId(SideId(1)), Number(6)],
        &[],
    );
    is(&mut b, "|start", "start", &[], &[]);
    is(&mut b, "|turn|1", "turn", &[Number(1)], &[]);
    is(&mut b, "|upkeep", "upkeep", &[], &[]);
    is(
        &mut b,
        "|win|Bob",
        "win",
        &[LogArg::PlayerName(SideId(1))],
        &[],
    );
    is(&mut b, "|tie", "tie", &[], &[]);
}

#[test]
fn switch_drag_replace_and_forme_lines() {
    let mut b = standard_active();
    // switch: ONE function part `details|health`, two views.
    split(
        &mut b,
        [
            "|split|p1",
            "|switch|p1a: Wugtrio|Wugtrio, L92, F|214/214",
            "|switch|p1a: Wugtrio|Wugtrio, L92, F|100/100",
        ],
        "switch",
        &[Mon(W), LogArg::FullDetails(W)],
        &[],
    );
    // `[from] ${sourceEffect}` is the Move's name (U-turn), not its fullname.
    split(
        &mut b,
        [
            "|split|p2",
            "|switch|p2a: Klefki|Klefki, L78, F|217/217|[from] U-turn",
            "|switch|p2a: Klefki|Klefki, L78, F|100/100|[from] U-turn",
        ],
        "switch",
        &[Mon(K), LogArg::FullDetails(K)],
        &[LogTag::Value("from", eff("U-turn"))],
    );
    // Reshiram is genderless (no gender field).
    split(
        &mut b,
        [
            "|split|p2",
            "|switch|p2b: Reshiram|Reshiram, L72|263/263",
            "|switch|p2b: Reshiram|Reshiram, L72|100/100",
        ],
        "switch",
        &[Mon(R), LogArg::FullDetails(R)],
        &[],
    );
    set_hp(&mut b, D, 251, 394, "par");
    split(
        &mut b,
        [
            "|split|p1",
            "|drag|p1b: Dondozo|Dondozo, L85, F|251/394 par",
            "|drag|p1b: Dondozo|Dondozo, L85, F|64/100 par",
        ],
        "drag",
        &[Mon(D), LogArg::FullDetails(D)],
        &[],
    );
    // detailschange (Details), including the tera suffix; replace is the plain getUpdatedDetails.
    let steel = dex::type_id("Steel").unwrap();
    b.state.pokemon[3].terastallized = steel;
    is(
        &mut b,
        "|detailschange|p1: Deoxys|Deoxys-Defense, L88, tera:Steel",
        "detailschange",
        &[Mon(MonId(3)), LogArg::Details(MonId(3))],
        &[],
    );
    b.state.pokemon[3].terastallized = TypeId::NONE;
    is(
        &mut b,
        "|detailschange|p1: Deoxys|Deoxys-Defense, L88|[silent]",
        "detailschange",
        &[Mon(MonId(3)), LogArg::Details(MonId(3))],
        &[LogTag::Bare("silent")],
    );
    is(
        &mut b,
        "|replace|p1: Zoroark|Zoroark, L84, M, shiny",
        "replace",
        &[Mon(MonId(4)), LogArg::Details(MonId(4))],
        &[],
    );
    // formechange: the species name and an EMPTY third field, optional [from]/[msg].
    let mimikyu = id(EffectKind::Species, "Mimikyu-Busted");
    is(
        &mut b,
        "|-formechange|p2a: Klefki|Mimikyu-Busted|",
        "-formechange",
        &[Mon(K), LogArg::Species(mimikyu), Empty],
        &[],
    );
    is(
        &mut b,
        "|-formechange|p2a: Klefki|Mimikyu-Busted||[from] ability: Disguise",
        "-formechange",
        &[Mon(K), LogArg::Species(mimikyu), Empty],
        &[from(EffectKind::Ability, "Disguise")],
    );
    is(
        &mut b,
        "|-formechange|p2a: Klefki|Mimikyu-Busted|[msg]",
        "-formechange",
        &[Mon(K), LogArg::Species(mimikyu)],
        &[LogTag::Bare("msg")],
    );
    is(
        &mut b,
        "|-transform|p2a: Klefki|p1a: Wugtrio",
        "-transform",
        &[Mon(K), Mon(W)],
        &[],
    );
    is(
        &mut b,
        "|-transform|p2a: Klefki|p1a: Wugtrio|[from] ability: Imposter",
        "-transform",
        &[Mon(K), Mon(W)],
        &[from(EffectKind::Ability, "Imposter")],
    );
    is(
        &mut b,
        "|-terastallize|p2b: Reshiram|Fire",
        "-terastallize",
        &[Mon(R), LogArg::Type(dex::type_id("Fire").unwrap())],
        &[],
    );
}

#[test]
fn damage_and_heal_splits() {
    let mut b = standard_active();
    let psn = EffectRef::Dex(id(EffectKind::Condition, "psn"));
    // HP variants: plain, fainted (`0 fnt`), with status word on both views.
    set_hp(&mut b, K, 27, 217, "");
    split(
        &mut b,
        [
            "|split|p2",
            "|-damage|p2a: Klefki|27/217",
            "|-damage|p2a: Klefki|13/100",
        ],
        "-damage",
        &[Mon(K), LogArg::Health(K)],
        &[],
    );
    set_hp(&mut b, W, 0, 214, "");
    split(
        &mut b,
        [
            "|split|p1",
            "|-damage|p1a: Wugtrio|0 fnt",
            "|-damage|p1a: Wugtrio|0 fnt",
        ],
        "-damage",
        &[Mon(W), LogArg::Health(W)],
        &[],
    );
    set_hp(&mut b, W, 141, 266, "par");
    split(
        &mut b,
        [
            "|split|p1",
            "|-damage|p1a: Wugtrio|141/266 par",
            "|-damage|p1a: Wugtrio|54/100 par",
        ],
        "-damage",
        &[Mon(W), LogArg::Health(W)],
        &[],
    );
    // [from] item / word / Name / [of] / [partiallytrapped] / pokemon: forms.
    set_hp(&mut b, R, 237, 263, "");
    split(
        &mut b,
        [
            "|split|p2",
            "|-damage|p2b: Reshiram|237/263|[from] item: Life Orb",
            "|-damage|p2b: Reshiram|91/100|[from] item: Life Orb",
        ],
        "-damage",
        &[Mon(R), LogArg::Health(R)],
        &[from(EffectKind::Item, "Life Orb")],
    );
    set_hp(&mut b, K, 216, 265, "brn");
    split(
        &mut b,
        [
            "|split|p2",
            "|-damage|p2a: Klefki|216/265 brn|[from] brn",
            "|-damage|p2a: Klefki|82/100 brn|[from] brn",
        ],
        "-damage",
        &[Mon(K), LogArg::Health(K)],
        &[LogTag::From(EffectRef::Dex(id(
            EffectKind::Condition,
            "brn",
        )))],
    );
    // tox damage is written `[from] psn` by the damage mutator (battle.ts:2150): `Value` with text.
    set_hp(&mut b, K, 221, 329, "tox");
    split(
        &mut b,
        [
            "|split|p2",
            "|-damage|p2a: Klefki|221/329 tox|[from] psn",
            "|-damage|p2a: Klefki|68/100 tox|[from] psn",
        ],
        "-damage",
        &[Mon(K), LogArg::Health(K)],
        &[LogTag::From(psn)],
    );
    set_hp(&mut b, D, 91, 394, "");
    split(
        &mut b,
        [
            "|split|p1",
            "|-damage|p1b: Dondozo|91/394|[from] Recoil",
            "|-damage|p1b: Dondozo|24/100|[from] Recoil",
        ],
        "-damage",
        &[Mon(D), LogArg::Health(D)],
        &[LogTag::From(EffectRef::Dex(id(
            EffectKind::Condition,
            "Recoil",
        )))],
    );
    split(
        &mut b,
        [
            "|split|p1",
            "|-damage|p1b: Dondozo|91/394|[from] recoil",
            "|-damage|p1b: Dondozo|24/100|[from] recoil",
        ],
        "-damage",
        &[Mon(D), LogArg::Health(D)],
        &[LogTag::Value("from", Text("recoil"))],
    );
    set_hp(&mut b, K, 238, 272, "");
    split(
        &mut b,
        [
            "|split|p2",
            "|-damage|p2a: Klefki|238/272|[from] Stealth Rock",
            "|-damage|p2a: Klefki|88/100|[from] Stealth Rock",
        ],
        "-damage",
        &[Mon(K), LogArg::Health(K)],
        &[LogTag::From(EffectRef::Dex(id(
            EffectKind::Condition,
            "Stealth Rock",
        )))],
    );
    set_hp(&mut b, R, 77, 246, "");
    split(
        &mut b,
        [
            "|split|p2",
            "|-damage|p2b: Reshiram|77/246|[from] Leech Seed|[of] p1a: Wugtrio",
            "|-damage|p2b: Reshiram|32/100|[from] Leech Seed|[of] p1a: Wugtrio",
        ],
        "-damage",
        &[Mon(R), LogArg::Health(R)],
        &[
            LogTag::From(EffectRef::Dex(id(EffectKind::Condition, "Leech Seed"))),
            LogTag::Of(W),
        ],
    );
    set_hp(&mut b, K, 219, 256, "");
    split(
        &mut b,
        [
            "|split|p2",
            "|-damage|p2a: Klefki|219/256|[from] move: Infestation|[partiallytrapped]",
            "|-damage|p2a: Klefki|86/100|[from] move: Infestation|[partiallytrapped]",
        ],
        "-damage",
        &[Mon(K), LogArg::Health(K)],
        &[
            from(EffectKind::Move, "Infestation"),
            LogTag::Bare("partiallytrapped"),
        ],
    );
    set_hp(&mut b, W, 137, 242, "");
    split(
        &mut b,
        [
            "|split|p1",
            "|-damage|p1a: Wugtrio|137/242|[from] item: Rocky Helmet|[of] p2b: Reshiram",
            "|-damage|p1a: Wugtrio|57/100|[from] item: Rocky Helmet|[of] p2b: Reshiram",
        ],
        "-damage",
        &[Mon(W), LogArg::Health(W)],
        &[from(EffectKind::Item, "Rocky Helmet"), LogTag::Of(R)],
    );
    set_hp(&mut b, W, 54, 272, "");
    split(
        &mut b,
        [
            "|split|p1",
            "|-damage|p1a: Wugtrio|54/272|[from] ability: Gulp Missile|[of] p1b: Dondozo",
            "|-damage|p1a: Wugtrio|20/100|[from] ability: Gulp Missile|[of] p1b: Dondozo",
        ],
        "-damage",
        &[Mon(W), LogArg::Health(W)],
        &[from(EffectKind::Ability, "Gulp Missile"), LogTag::Of(D)],
    );
    set_hp(&mut b, R, 174, 224, "");
    split(
        &mut b,
        [
            "|split|p2",
            "|-damage|p2b: Reshiram|174/224|[from] pokemon: Mimikyu-Busted",
            "|-damage|p2b: Reshiram|78/100|[from] pokemon: Mimikyu-Busted",
        ],
        "-damage",
        &[Mon(R), LogArg::Health(R)],
        &[from(EffectKind::Species, "Mimikyu-Busted")],
    );

    // -heal: plain, [from] item / ability / drain / Wish / Revival Blessing (NOSLOT) / silent.
    set_hp(&mut b, K, 200, 217, "");
    split(
        &mut b,
        [
            "|split|p2",
            "|-heal|p2a: Klefki|200/217",
            "|-heal|p2a: Klefki|93/100",
        ],
        "-heal",
        &[Mon(K), LogArg::Health(K)],
        &[],
    );
    split(
        &mut b,
        [
            "|split|p2",
            "|-heal|p2a: Klefki|200/217|[from] item: Leftovers",
            "|-heal|p2a: Klefki|93/100|[from] item: Leftovers",
        ],
        "-heal",
        &[Mon(K), LogArg::Health(K)],
        &[from(EffectKind::Item, "Leftovers")],
    );
    split(
        &mut b,
        [
            "|split|p2",
            "|-heal|p2a: Klefki|200/217|[from] ability: Poison Heal",
            "|-heal|p2a: Klefki|93/100|[from] ability: Poison Heal",
        ],
        "-heal",
        &[Mon(K), LogArg::Health(K)],
        &[from(EffectKind::Ability, "Poison Heal")],
    );
    split(
        &mut b,
        [
            "|split|p2",
            "|-heal|p2a: Klefki|200/217|[from] ability: Water Absorb|[of] p1a: Wugtrio",
            "|-heal|p2a: Klefki|93/100|[from] ability: Water Absorb|[of] p1a: Wugtrio",
        ],
        "-heal",
        &[Mon(K), LogArg::Health(K)],
        &[from(EffectKind::Ability, "Water Absorb"), LogTag::Of(W)],
    );
    split(
        &mut b,
        [
            "|split|p2",
            "|-heal|p2a: Klefki|200/217|[from] drain|[of] p1a: Wugtrio",
            "|-heal|p2a: Klefki|93/100|[from] drain|[of] p1a: Wugtrio",
        ],
        "-heal",
        &[Mon(K), LogArg::Health(K)],
        &[LogTag::Value("from", Text("drain")), LogTag::Of(W)],
    );
    split(
        &mut b,
        [
            "|split|p2",
            "|-heal|p2a: Klefki|200/217|[silent]",
            "|-heal|p2a: Klefki|93/100|[silent]",
        ],
        "-heal",
        &[Mon(K), LogArg::Health(K)],
        &[LogTag::Bare("silent")],
    );
    split(
        &mut b,
        [
            "|split|p2",
            "|-heal|p2a: Klefki|200/217|[from] move: Wish|[wisher] Vaporeon",
            "|-heal|p2a: Klefki|93/100|[from] move: Wish|[wisher] Vaporeon",
        ],
        "-heal",
        &[Mon(K), LogArg::Health(K)],
        &[
            from(EffectKind::Move, "Wish"),
            LogTag::Value("wisher", Text("Vaporeon")),
        ],
    );
    // Revival Blessing heals a benched (NOSLOT) mon.
    set_hp(&mut b, GHOLDENGO, 132, 264, "");
    split(
        &mut b,
        [
            "|split|p1",
            "|-heal|p1: Gholdengo|132/264|[from] move: Revival Blessing",
            "|-heal|p1: Gholdengo|50/100|[from] move: Revival Blessing",
        ],
        "-heal",
        &[Mon(GHOLDENGO), LogArg::Health(GHOLDENGO)],
        &[from(EffectKind::Move, "Revival Blessing")],
    );
}

#[test]
fn moves_and_hit_results() {
    let mut b = standard_active();
    is(
        &mut b,
        "|-miss|p1a: Wugtrio|p2a: Klefki",
        "-miss",
        &[Mon(W), Mon(K)],
        &[],
    );
    is(&mut b, "|-fail|p1a: Wugtrio", "-fail", &[Mon(W)], &[]);
    is(
        &mut b,
        "|-fail|p2a: Klefki|heal",
        "-fail",
        &[Mon(K), Text("heal")],
        &[],
    );
    is(
        &mut b,
        "|-fail|p2a: Klefki|par",
        "-fail",
        &[Mon(K), LogArg::Status(Status::Paralysis)],
        &[],
    );
    is(
        &mut b,
        "|-fail|p2a: Klefki|tox",
        "-fail",
        &[Mon(K), LogArg::Status(Status::Toxic)],
        &[],
    );
    is(
        &mut b,
        "|-fail|p1a: Wugtrio|unboost|[from] ability: Clear Body|[of] p1a: Wugtrio",
        "-fail",
        &[Mon(W), Text("unboost")],
        &[from(EffectKind::Ability, "Clear Body"), LogTag::Of(W)],
    );
    is(
        &mut b,
        "|-fail|p1a: Wugtrio|unboost|atk|[from] ability: Clear Body|[of] p1a: Wugtrio",
        "-fail",
        &[Mon(W), Text("unboost"), Text("atk")],
        &[from(EffectKind::Ability, "Clear Body"), LogTag::Of(W)],
    );
    is(
        &mut b,
        "|-fail|p1a: Wugtrio|move: Substitute",
        "-fail",
        &[Mon(W), full(EffectKind::Move, "Substitute")],
        &[],
    );
    is(
        &mut b,
        "|-fail|p1a: Wugtrio|move: Substitute|[weak]",
        "-fail",
        &[Mon(W), full(EffectKind::Move, "Substitute")],
        &[LogTag::Bare("weak")],
    );
    is(
        &mut b,
        "|-fail|p1a: Wugtrio|unboost|[from] item: Clear Amulet|[of] p1a: Wugtrio",
        "-fail",
        &[Mon(W), Text("unboost")],
        &[from(EffectKind::Item, "Clear Amulet"), LogTag::Of(W)],
    );
    is(&mut b, "|-immune|p2a: Klefki", "-immune", &[Mon(K)], &[]);
    is(
        &mut b,
        "|-immune|p2a: Klefki|[from] ability: Levitate",
        "-immune",
        &[Mon(K)],
        &[from(EffectKind::Ability, "Levitate")],
    );
    for cmd in ["-crit", "-supereffective", "-resisted"] {
        let want = format!("|{cmd}|p2b: Reshiram");
        is(&mut b, &want, cmd, &[Mon(R)], &[]);
    }
    // -hitcount prints the target at the time: NOSLOT once it fainted.
    is(
        &mut b,
        "|-hitcount|p2b: Reshiram|3",
        "-hitcount",
        &[Mon(R), Number(3)],
        &[],
    );
    is(
        &mut b,
        "|-hitcount|p1: Gholdengo|2",
        "-hitcount",
        &[Mon(GHOLDENGO), Number(2)],
        &[],
    );
    is(
        &mut b,
        "|-prepare|p1b: Dondozo|Solar Beam",
        "-prepare",
        &[Mon(D), eff("Solar Beam")],
        &[],
    );
    is(
        &mut b,
        "|-mustrecharge|p1a: Wugtrio",
        "-mustrecharge",
        &[Mon(W)],
        &[],
    );
    // -anim: through addMove, same pointer as `move`.
    b.add_move(LogEntry::new(
        "-anim",
        &[Mon(D), Text("Meteor Beam"), Mon(K)],
        &[],
    ));
    assert_eq!(last(&b), "|-anim|p1b: Dondozo|Meteor Beam|p2a: Klefki");
    // faint
    is(&mut b, "|faint|p2a: Klefki", "faint", &[Mon(K)], &[]);
    // cant
    is(
        &mut b,
        "|cant|p1b: Dondozo|par",
        "cant",
        &[Mon(D), Text("par")],
        &[],
    );
    is(
        &mut b,
        "|cant|p1b: Dondozo|ability: Truant",
        "cant",
        &[Mon(D), full(EffectKind::Ability, "Truant")],
        &[],
    );
    is(
        &mut b,
        "|cant|p2a: Klefki|move: Taunt|Calm Mind",
        "cant",
        &[Mon(K), full(EffectKind::Move, "Taunt"), eff("Calm Mind")],
        &[],
    );
    is(
        &mut b,
        "|cant|p2b: Reshiram|ability: Armor Tail|Bullet Punch|[of] p1b: Dondozo",
        "cant",
        &[
            Mon(R),
            full(EffectKind::Ability, "Armor Tail"),
            eff("Bullet Punch"),
        ],
        &[LogTag::Of(D)],
    );
    is(
        &mut b,
        "|cant|p1b: Dondozo|Disable|Swords Dance",
        "cant",
        &[Mon(D), Text("Disable"), eff("Swords Dance")],
        &[],
    );
}

#[test]
fn stat_stages() {
    let mut b = standard_active();
    is(
        &mut b,
        "|-boost|p1b: Dondozo|spa|1",
        "-boost",
        &[Mon(D), Text("spa"), Number(1)],
        &[],
    );
    is(
        &mut b,
        "|-boost|p1b: Dondozo|atk|0",
        "-boost",
        &[Mon(D), Text("atk"), Number(0)],
        &[],
    );
    is(
        &mut b,
        "|-boost|p2a: Klefki|spa|1|[from] item: Throat Spray",
        "-boost",
        &[Mon(K), Text("spa"), Number(1)],
        &[from(EffectKind::Item, "Throat Spray")],
    );
    is(
        &mut b,
        "|-unboost|p2b: Reshiram|spe|1",
        "-unboost",
        &[Mon(R), Text("spe"), Number(1)],
        &[],
    );
    // `-boostBy` of 0 stages prints 0 (JS -0).
    is(
        &mut b,
        "|-unboost|p2b: Reshiram|spe|0",
        "-unboost",
        &[Mon(R), Text("spe"), Number(0)],
        &[],
    );
    is(
        &mut b,
        "|-setboost|p1a: Wugtrio|atk|6|[from] move: Belly Drum",
        "-setboost",
        &[Mon(W), Text("atk"), Number(6)],
        &[from(EffectKind::Move, "Belly Drum")],
    );
    is(
        &mut b,
        "|-clearboost|p1a: Wugtrio",
        "-clearboost",
        &[Mon(W)],
        &[],
    );
    is(&mut b, "|-clearallboost", "-clearallboost", &[], &[]);
    is(
        &mut b,
        "|-clearnegativeboost|p1a: Wugtrio|[silent]",
        "-clearnegativeboost",
        &[Mon(W)],
        &[LogTag::Bare("silent")],
    );
    is(
        &mut b,
        "|-ability|p2b: Reshiram|Intimidate|boost",
        "-ability",
        &[Mon(R), eff("Intimidate"), Text("boost")],
        &[],
    );
    is(
        &mut b,
        "|-ability|p2b: Reshiram|Turboblaze",
        "-ability",
        &[Mon(R), eff("Turboblaze")],
        &[],
    );
    is(
        &mut b,
        "|-ability|p1a: Wugtrio|Tablets of Ruin|Trace|[from] ability: Trace|[of] p2b: Reshiram",
        "-ability",
        &[Mon(W), eff("Tablets of Ruin"), eff("Trace")],
        &[from(EffectKind::Ability, "Trace"), LogTag::Of(R)],
    );
}

#[test]
fn status_and_volatiles() {
    let mut b = standard_active();
    is(
        &mut b,
        "|-status|p1a: Wugtrio|brn",
        "-status",
        &[Mon(W), LogArg::Status(Status::Burn)],
        &[],
    );
    is(
        &mut b,
        "|-status|p1a: Wugtrio|slp",
        "-status",
        &[Mon(W), Text("slp")],
        &[],
    );
    is(
        &mut b,
        "|-status|p1a: Wugtrio|par|[from] move: Thunder Wave",
        "-status",
        &[Mon(W), LogArg::Status(Status::Paralysis)],
        &[from(EffectKind::Move, "Thunder Wave")],
    );
    is(
        &mut b,
        "|-status|p1a: Wugtrio|par|[from] ability: Static|[of] p2b: Reshiram",
        "-status",
        &[Mon(W), LogArg::Status(Status::Paralysis)],
        &[from(EffectKind::Ability, "Static"), LogTag::Of(R)],
    );
    is(
        &mut b,
        "|-status|p1a: Wugtrio|tox|[from] item: Toxic Orb",
        "-status",
        &[Mon(W), LogArg::Status(Status::Toxic)],
        &[from(EffectKind::Item, "Toxic Orb")],
    );
    is(
        &mut b,
        "|-curestatus|p1a: Wugtrio|par|[msg]",
        "-curestatus",
        &[Mon(W), LogArg::Status(Status::Paralysis)],
        &[LogTag::Bare("msg")],
    );
    is(
        &mut b,
        "|-curestatus|p1a: Wugtrio|slp|[from] ability: Natural Cure|[silent]",
        "-curestatus",
        &[Mon(W), LogArg::Status(Status::Sleep)],
        &[
            from(EffectKind::Ability, "Natural Cure"),
            LogTag::Bare("silent"),
        ],
    );
    is(
        &mut b,
        "|-curestatus|p1a: Wugtrio|frz|[from] move: Flare Blitz",
        "-curestatus",
        &[Mon(W), LogArg::Status(Status::Freeze)],
        &[from(EffectKind::Move, "Flare Blitz")],
    );
    // -start (14 shapes): name, move:, word, silent, [of], two names + [from]/[of], typechange...
    is(
        &mut b,
        "|-start|p1a: Wugtrio|confusion",
        "-start",
        &[Mon(W), Text("confusion")],
        &[],
    );
    is(
        &mut b,
        "|-start|p1a: Wugtrio|Encore",
        "-start",
        &[Mon(W), Text("Encore")],
        &[],
    );
    is(
        &mut b,
        "|-start|p1a: Wugtrio|Throat Chop|[silent]",
        "-start",
        &[Mon(W), Text("Throat Chop")],
        &[LogTag::Bare("silent")],
    );
    is(
        &mut b,
        "|-start|p1a: Wugtrio|move: Taunt",
        "-start",
        &[Mon(W), full(EffectKind::Move, "Taunt")],
        &[],
    );
    is(
        &mut b,
        "|-start|p1a: Wugtrio|move: Yawn|[of] p2b: Reshiram",
        "-start",
        &[Mon(W), full(EffectKind::Move, "Yawn")],
        &[LogTag::Of(R)],
    );
    is(
        &mut b,
        "|-start|p1a: Wugtrio|ability: Flash Fire",
        "-start",
        &[Mon(W), full(EffectKind::Ability, "Flash Fire")],
        &[],
    );
    is(
        &mut b,
        "|-start|p1a: Wugtrio|Disable|Swords Dance|[from] ability: Cursed Body|[of] p2b: Reshiram",
        "-start",
        &[Mon(W), Text("Disable"), eff("Swords Dance")],
        &[from(EffectKind::Ability, "Cursed Body"), LogTag::Of(R)],
    );
    is(
        &mut b,
        "|-start|p1a: Wugtrio|typechange|Fire|[from] ability: Protean",
        "-start",
        &[
            Mon(W),
            Text("typechange"),
            LogArg::Type(dex::type_id("Fire").unwrap()),
        ],
        &[from(EffectKind::Ability, "Protean")],
    );
    is(
        &mut b,
        "|-start|p1a: Wugtrio|typechange|???/Fighting|[from] move: Double Shock",
        "-start",
        &[
            Mon(W),
            Text("typechange"),
            LogArg::Parts(&[
                LogArg::Type(dex::type_id("???").unwrap()),
                Text("/"),
                LogArg::Type(dex::type_id("Fighting").unwrap()),
            ]),
        ],
        &[from(EffectKind::Move, "Double Shock")],
    );
    is(
        &mut b,
        "|-start|p1a: Wugtrio|typeadd|Ghost|[silent]",
        "-start",
        &[
            Mon(W),
            Text("typeadd"),
            LogArg::Type(dex::type_id("Ghost").unwrap()),
        ],
        &[LogTag::Bare("silent")],
    );
    is(
        &mut b,
        "|-start|p1a: Wugtrio|protosynthesisspa",
        "-start",
        &[Mon(W), Text("protosynthesisspa")],
        &[],
    );
    is(
        &mut b,
        "|-start|p1a: Wugtrio|confusion|[from] ability: Poison Puppeteer|[of] p2b: Reshiram",
        "-start",
        &[Mon(W), Text("confusion")],
        &[from(EffectKind::Ability, "Poison Puppeteer"), LogTag::Of(R)],
    );
    is(
        &mut b,
        "|-start|p1a: Wugtrio|Substitute|[from] move: Shed Tail",
        "-start",
        &[Mon(W), Text("Substitute")],
        &[from(EffectKind::Move, "Shed Tail")],
    );
    // -end
    is(
        &mut b,
        "|-end|p1a: Wugtrio|Protosynthesis|[silent]",
        "-end",
        &[Mon(W), Text("Protosynthesis")],
        &[LogTag::Bare("silent")],
    );
    is(
        &mut b,
        "|-end|p1a: Wugtrio|Illusion",
        "-end",
        &[Mon(W), Text("Illusion")],
        &[],
    );
    is(
        &mut b,
        "|-end|p1a: Wugtrio|move: Taunt",
        "-end",
        &[Mon(W), full(EffectKind::Move, "Taunt")],
        &[],
    );
    is(
        &mut b,
        "|-end|p1a: Wugtrio|move: Taunt|[silent]",
        "-end",
        &[Mon(W), full(EffectKind::Move, "Taunt")],
        &[LogTag::Bare("silent")],
    );
    is(
        &mut b,
        "|-end|p1a: Wugtrio|confusion",
        "-end",
        &[Mon(W), Text("confusion")],
        &[],
    );
    is(
        &mut b,
        "|-end|p1a: Wugtrio|Infestation|[partiallytrapped]",
        "-end",
        &[Mon(W), Text("Infestation")],
        &[LogTag::Bare("partiallytrapped")],
    );
    is(
        &mut b,
        "|-end|p1a: Wugtrio|Infestation|[partiallytrapped]|[silent]",
        "-end",
        &[Mon(W), Text("Infestation")],
        &[LogTag::Bare("partiallytrapped"), LogTag::Bare("silent")],
    );
    is(
        &mut b,
        "|-end|p1a: Wugtrio|ability: Slow Start|[silent]",
        "-end",
        &[Mon(W), full(EffectKind::Ability, "Slow Start")],
        &[LogTag::Bare("silent")],
    );
    // -activate (9)
    is(
        &mut b,
        "|-activate|p1a: Wugtrio|move: Protect",
        "-activate",
        &[Mon(W), full(EffectKind::Move, "Protect")],
        &[],
    );
    is(
        &mut b,
        "|-activate|p1a: Wugtrio|ability: Storm Drain",
        "-activate",
        &[Mon(W), full(EffectKind::Ability, "Storm Drain")],
        &[],
    );
    is(
        &mut b,
        "|-activate|p1a: Wugtrio|confusion",
        "-activate",
        &[Mon(W), Text("confusion")],
        &[],
    );
    is(
        &mut b,
        "|-activate|p1a: Wugtrio|trapped",
        "-activate",
        &[Mon(W), Text("trapped")],
        &[],
    );
    is(
        &mut b,
        "|-activate|p2a: Klefki|move: Poltergeist|Choice Specs",
        "-activate",
        &[
            Mon(K),
            full(EffectKind::Move, "Poltergeist"),
            eff("Choice Specs"),
        ],
        &[],
    );
    is(
        &mut b,
        "|-activate|p2a: Klefki|move: Infestation|[of] p1b: Dondozo",
        "-activate",
        &[Mon(K), full(EffectKind::Move, "Infestation")],
        &[LogTag::Of(D)],
    );
    is(
        &mut b,
        "|-activate|p2b: Reshiram|ability: Quark Drive|[fromitem]",
        "-activate",
        &[Mon(R), full(EffectKind::Ability, "Quark Drive")],
        &[LogTag::Bare("fromitem")],
    );
    is(
        &mut b,
        "|-activate|p1a: Wugtrio|Orichalcum Pulse|[source]",
        "-activate",
        &[Mon(W), Text("Orichalcum Pulse")],
        &[LogTag::Bare("source")],
    );
    is(
        &mut b,
        "|-activate|p2a: Klefki|move: Substitute|[damage]",
        "-activate",
        &[Mon(K), full(EffectKind::Move, "Substitute")],
        &[LogTag::Bare("damage")],
    );
    is(
        &mut b,
        "|-activate|p2a: Klefki|move: Phantom Force|[broken]",
        "-activate",
        &[Mon(K), full(EffectKind::Move, "Phantom Force")],
        &[LogTag::Bare("broken")],
    );
    is(
        &mut b,
        "|-block|p1a: Wugtrio|ability: Aroma Veil|[of] p1b: Dondozo",
        "-block",
        &[Mon(W), full(EffectKind::Ability, "Aroma Veil")],
        &[LogTag::Of(D)],
    );
    // -singleturn / -singlemove (4 + 1)
    is(
        &mut b,
        "|-singleturn|p1a: Wugtrio|Protect",
        "-singleturn",
        &[Mon(W), Text("Protect")],
        &[],
    );
    is(
        &mut b,
        "|-singleturn|p1a: Wugtrio|move: Protect",
        "-singleturn",
        &[Mon(W), full(EffectKind::Move, "Protect")],
        &[],
    );
    is(
        &mut b,
        "|-singleturn|p1b: Dondozo|Helping Hand|[of] p1a: Wugtrio",
        "-singleturn",
        &[Mon(D), Text("Helping Hand")],
        &[LogTag::Of(W)],
    );
    is(
        &mut b,
        "|-singleturn|p1b: Dondozo|move: Instruct|[of] p1a: Wugtrio",
        "-singleturn",
        &[Mon(D), full(EffectKind::Move, "Instruct")],
        &[LogTag::Of(W)],
    );
    is(
        &mut b,
        "|-singlemove|p1a: Wugtrio|Destiny Bond|[silent]",
        "-singlemove",
        &[Mon(W), Text("Destiny Bond")],
        &[LogTag::Bare("silent")],
    );
}

#[test]
fn items_and_abilities() {
    let mut b = standard_active();
    is(
        &mut b,
        "|-enditem|p1a: Wugtrio|Sitrus Berry|[eat]",
        "-enditem",
        &[Mon(W), eff("Sitrus Berry")],
        &[LogTag::Bare("eat")],
    );
    is(
        &mut b,
        "|-enditem|p1a: Wugtrio|White Herb",
        "-enditem",
        &[Mon(W), eff("White Herb")],
        &[],
    );
    is(
        &mut b,
        "|-enditem|p1a: Wugtrio|Choice Band|[from] move: Knock Off|[of] p2b: Reshiram",
        "-enditem",
        &[Mon(W), eff("Choice Band")],
        &[from(EffectKind::Move, "Knock Off"), LogTag::Of(R)],
    );
    is(
        &mut b,
        "|-enditem|p1a: Wugtrio|Choice Band|[silent]|[from] move: Trick",
        "-enditem",
        &[Mon(W), eff("Choice Band")],
        &[LogTag::Bare("silent"), from(EffectKind::Move, "Trick")],
    );
    is(
        &mut b,
        "|-enditem|p1a: Wugtrio|Sitrus Berry|[from] stealeat|[move] Bug Bite|[of] p2b: Reshiram",
        "-enditem",
        &[Mon(W), eff("Sitrus Berry")],
        &[
            LogTag::Value("from", Text("stealeat")),
            LogTag::Value("move", Text("Bug Bite")),
            LogTag::Of(R),
        ],
    );
    is(
        &mut b,
        "|-enditem|p1a: Wugtrio|Sitrus Berry|[from] move: Incinerate",
        "-enditem",
        &[Mon(W), eff("Sitrus Berry")],
        &[from(EffectKind::Move, "Incinerate")],
    );
    is(
        &mut b,
        "|-enditem|p1a: Wugtrio|Leftovers|[silent]|[from] ability: Pickpocket|[of] p2b: Reshiram",
        "-enditem",
        &[Mon(W), eff("Leftovers")],
        &[
            LogTag::Bare("silent"),
            from(EffectKind::Ability, "Pickpocket"),
            LogTag::Of(R),
        ],
    );
    is(
        &mut b,
        "|-item|p1a: Wugtrio|Leftovers|[from] ability: Frisk|[of] p2b: Reshiram",
        "-item",
        &[Mon(W), eff("Leftovers")],
        &[from(EffectKind::Ability, "Frisk"), LogTag::Of(R)],
    );
    is(
        &mut b,
        "|-item|p1a: Wugtrio|Leftovers|[from] move: Trick",
        "-item",
        &[Mon(W), eff("Leftovers")],
        &[from(EffectKind::Move, "Trick")],
    );
    is(
        &mut b,
        "|-item|p1a: Wugtrio|Leftovers|[from] ability: Harvest",
        "-item",
        &[Mon(W), eff("Leftovers")],
        &[from(EffectKind::Ability, "Harvest")],
    );
    // Magician steals from a benched (fainted) mon: [of] prints the NOSLOT ident.
    is(
        &mut b,
        "|-item|p1a: Wugtrio|Leftovers|[from] ability: Magician|[of] p1: Gholdengo",
        "-item",
        &[Mon(W), eff("Leftovers")],
        &[from(EffectKind::Ability, "Magician"), LogTag::Of(GHOLDENGO)],
    );
}

#[test]
fn field_weather_and_sides() {
    let mut b = standard_active();
    is(
        &mut b,
        "|-weather|RainDance|[upkeep]",
        "-weather",
        &[Text("RainDance")],
        &[LogTag::Bare("upkeep")],
    );
    is(
        &mut b,
        "|-weather|SunnyDay",
        "-weather",
        &[LogArg::Effect(fx(EffectKind::Condition, "SunnyDay"))],
        &[],
    );
    is(
        &mut b,
        "|-weather|Sandstorm|[from] ability: Sand Stream|[of] p1a: Wugtrio",
        "-weather",
        &[LogArg::Effect(fx(EffectKind::Condition, "Sandstorm"))],
        &[from(EffectKind::Ability, "Sand Stream"), LogTag::Of(W)],
    );
    is(&mut b, "|-weather|none", "-weather", &[Text("none")], &[]);
    is(
        &mut b,
        "|-fieldstart|move: Trick Room|[of] p2b: Reshiram",
        "-fieldstart",
        &[full(EffectKind::Move, "Trick Room")],
        &[LogTag::Of(R)],
    );
    is(
        &mut b,
        "|-fieldstart|move: Grassy Terrain|[from] ability: Grassy Surge|[of] p2b: Reshiram",
        "-fieldstart",
        &[Text("move: Grassy Terrain")],
        &[from(EffectKind::Ability, "Grassy Surge"), LogTag::Of(R)],
    );
    is(
        &mut b,
        "|-fieldend|move: Trick Room",
        "-fieldend",
        &[full(EffectKind::Move, "Trick Room")],
        &[],
    );
    // Side conditions: a Side prints `pN: PlayerName`; the literal is chosen per call site.
    is(
        &mut b,
        "|-sidestart|p2: Bob|Reflect",
        "-sidestart",
        &[LogArg::Side(SideId(1)), Text("Reflect")],
        &[],
    );
    is(
        &mut b,
        "|-sidestart|p2: Bob|move: Light Screen",
        "-sidestart",
        &[
            LogArg::Side(SideId(1)),
            full(EffectKind::Move, "Light Screen"),
        ],
        &[],
    );
    is(
        &mut b,
        "|-sideend|p1: Alice|move: Tailwind",
        "-sideend",
        &[LogArg::Side(SideId(0)), full(EffectKind::Move, "Tailwind")],
        &[],
    );
    is(
        &mut b,
        "|-sideend|p1: Alice|Spikes",
        "-sideend",
        &[LogArg::Side(SideId(0)), Text("Spikes")],
        &[],
    );
    is(
        &mut b,
        "|-sideend|p1: Alice|move: Toxic Spikes|[of] p2b: Reshiram",
        "-sideend",
        &[
            LogArg::Side(SideId(0)),
            full(EffectKind::Move, "Toxic Spikes"),
        ],
        &[LogTag::Of(R)],
    );
    is(
        &mut b,
        "|-sideend|p1: Alice|Stealth Rock|[from] move: Rapid Spin|[of] p1a: Wugtrio",
        "-sideend",
        &[LogArg::Side(SideId(0)), Text("Stealth Rock")],
        &[from(EffectKind::Move, "Rapid Spin"), LogTag::Of(W)],
    );
    is(
        &mut b,
        "|-swapsideconditions",
        "-swapsideconditions",
        &[],
        &[],
    );
    // Free text copied verbatim (UTF-8, ASCII apostrophe).
    is(
        &mut b,
        "|-hint|Illusion Level Mod is active, so this Pokémon's true level was hidden.",
        "-hint",
        &[Text(
            "Illusion Level Mod is active, so this Pokémon's true level was hidden.",
        )],
        &[],
    );
    is(
        &mut b,
        "|-message|Sleep Clause Mod activated.",
        "-message",
        &[Text("Sleep Clause Mod activated.")],
        &[],
    );
}
