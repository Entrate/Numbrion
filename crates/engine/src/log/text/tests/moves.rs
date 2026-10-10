//! `addMove` / `attrLastMove` / `retargetLastMove` semantics (battle.ts:3115-3144).
use super::*;

fn p1a() -> MonId {
    mon(0, 0) // Wugtrio
}
fn p1b() -> MonId {
    mon(0, 1) // Dondozo
}
fn p2a() -> MonId {
    mon(1, 0) // Klefki
}
fn p2b() -> MonId {
    mon(1, 1) // Reshiram
}

/// `|move|user|name|target` through `addMove`.
fn add_move(b: &mut Battle<TextLog>, user: MonId, name: &str, target: MonId, tags: &[LogTag<'_>]) {
    b.add_move(LogEntry::new(
        "move",
        &[LogArg::Mon(user), LogArg::Text(name), LogArg::Mon(target)],
        tags,
    ));
}

#[test]
fn plain_move_line_and_source_tags() {
    let mut b = standard_active();
    add_move(&mut b, p1a(), "Stomping Tantrum", p2a(), &[]);
    assert_eq!(last(&b), "|move|p1a: Wugtrio|Stomping Tantrum|p2a: Klefki");
    // `[from] ${sourceEffect.fullname}` (battle-actions.ts:452): condition, ability, move.
    add_move(&mut b, p1b(), "Meteor Beam", p2b(), &[LogTag::From(fx(EffectKind::Condition, "lockedmove"))]);
    assert_eq!(last(&b), "|move|p1b: Dondozo|Meteor Beam|p2b: Reshiram|[from] lockedmove");
    add_move(&mut b, p2a(), "Swords Dance", p2a(), &[LogTag::From(fx(EffectKind::Ability, "Dancer"))]);
    assert_eq!(last(&b), "|move|p2a: Klefki|Swords Dance|p2a: Klefki|[from] ability: Dancer");
    // Tera Blast animation tag (data/moves.ts:19221): `[anim] Tera Blast Ground`.
    let ground = dex::type_id("Ground").unwrap();
    add_move(
        &mut b,
        p2b(),
        "Tera Blast",
        p1a(),
        &[LogTag::Value(
            "anim",
            LogArg::Parts(&[LogArg::Text("Tera Blast "), LogArg::Type(ground)]),
        )],
    );
    assert_eq!(last(&b), "|move|p2b: Reshiram|Tera Blast|p1a: Wugtrio|[anim] Tera Blast Ground");
}

#[test]
fn still_blanks_the_target_and_appends() {
    let mut b = standard_active();
    add_move(&mut b, p1a(), "Protect", p1a(), &[]);
    b.attr_last_move(MoveLineEdit::Still);
    assert_eq!(last(&b), "|move|p1a: Wugtrio|Protect||[still]");
    // Repeated attrs append repeatedly (Life Dew, battle-actions.ts:1192-1193), with a final empty spread.
    add_move(&mut b, p1a(), "Life Dew", p1a(), &[]);
    b.attr_last_move(MoveLineEdit::Still);
    b.attr_last_move(MoveLineEdit::Still);
    b.attr_last_move(MoveLineEdit::Tag(LogTag::Value(
        "spread",
        LogArg::Spread { mons: [MonId::NONE; 4], len: 0 },
    )));
    assert_eq!(last(&b), "|move|p1a: Wugtrio|Life Dew||[still]|[still]|[spread] ");
    // `[from]` before `[still]`: Magic Bounce reflecting a status move.
    add_move(&mut b, p2a(), "Stealth Rock", p1a(), &[LogTag::From(fx(EffectKind::Ability, "Magic Bounce"))]);
    b.attr_last_move(MoveLineEdit::Still);
    assert_eq!(last(&b), "|move|p2a: Klefki|Stealth Rock||[from] ability: Magic Bounce|[still]");
    // `[still]` can also arrive as a plain tag.
    add_move(&mut b, p2b(), "Tailwind", p2b(), &[]);
    b.attr_last_move(MoveLineEdit::Tag(LogTag::Bare("still")));
    assert_eq!(last(&b), "|move|p2b: Reshiram|Tailwind||[still]");
    add_move(&mut b, p2b(), "Tailwind", p2b(), &[]);
    b.attr_last_move(MoveLineEdit::Tag(LogTag::Text("[still]")));
    assert_eq!(last(&b), "|move|p2b: Reshiram|Tailwind||[still]");
}

#[test]
fn miss_notarget_and_spread() {
    let mut b = standard_active();
    add_move(&mut b, p2a(), "Thunder Wave", p1a(), &[]);
    b.attr_last_move(MoveLineEdit::Miss);
    assert_eq!(last(&b), "|move|p2a: Klefki|Thunder Wave|p1a: Wugtrio|[miss]");
    add_move(&mut b, p2b(), "Meteor Beam", p1b(), &[LogTag::From(fx(EffectKind::Condition, "lockedmove"))]);
    b.attr_last_move(MoveLineEdit::Miss);
    assert_eq!(last(&b), "|move|p2b: Reshiram|Meteor Beam|p1b: Dondozo|[from] lockedmove|[miss]");
    // A fainted/benched target prints as a NOSLOT ident (battle-actions.ts:457-462).
    add_move(&mut b, p1b(), "Body Press", mon(0, 2), &[]);
    b.attr_last_move(MoveLineEdit::NoTarget);
    assert_eq!(last(&b), "|move|p1b: Dondozo|Body Press|p1: Gholdengo|[notarget]");
    // [spread] lists the targets still hit, in getMoveTargets order (allies first).
    add_move(&mut b, p2a(), "Earthquake", p1a(), &[]);
    let hit = [p2b(), p1a(), p1b(), MonId::NONE];
    b.attr_last_move(MoveLineEdit::Tag(LogTag::Value("spread", LogArg::Spread { mons: hit, len: 3 })));
    assert_eq!(last(&b), "|move|p2a: Klefki|Earthquake|p1a: Wugtrio|[spread] p2b,p1a,p1b");
    add_move(&mut b, p1a(), "Icy Wind", p2a(), &[]);
    b.attr_last_move(MoveLineEdit::Tag(LogTag::Value(
        "spread",
        LogArg::Spread { mons: [p2b(), MonId::NONE, MonId::NONE, MonId::NONE], len: 1 },
    )));
    assert_eq!(last(&b), "|move|p1a: Wugtrio|Icy Wind|p2a: Klefki|[spread] p2b");
}

#[test]
fn packed_spread_edit() {
    // count in bits 0-1, then up to three field positions (side + 2 * slot) in two-bit fields.
    let mut b = standard_active();
    for (packed, want) in [
        (0u8, "[spread] "),
        (1 | 1 << 2, "[spread] p2a"),
        (2 | 1 << 2 | 3 << 4, "[spread] p2a,p2b"),
        (3 | 3 << 2 | 0 << 4 | 2 << 6, "[spread] p2b,p1a,p1b"),
    ] {
        add_move(&mut b, p1a(), "Heat Wave", p2a(), &[]);
        b.attr_last_move(MoveLineEdit::Spread(packed));
        assert_eq!(last(&b), format!("|move|p1a: Wugtrio|Heat Wave|p2a: Klefki|{want}"));
    }
}

#[test]
fn retarget_rewrites_field_four() {
    let mut b = standard_active();
    add_move(&mut b, p1a(), "Thunderbolt", p2a(), &[]);
    b.attr_last_move(MoveLineEdit::Retarget(p2b()));
    assert_eq!(last(&b), "|move|p1a: Wugtrio|Thunderbolt|p2b: Reshiram");
    // Tags already appended are preserved.
    add_move(&mut b, p1a(), "Thunderbolt", p2a(), &[LogTag::From(fx(EffectKind::Condition, "lockedmove"))]);
    b.attr_last_move(MoveLineEdit::Retarget(p2b()));
    assert_eq!(last(&b), "|move|p1a: Wugtrio|Thunderbolt|p2b: Reshiram|[from] lockedmove");
    // retargetLastMove has no [still] special case: it just overwrites the field.
    add_move(&mut b, p1a(), "Protect", p1a(), &[]);
    b.attr_last_move(MoveLineEdit::Still);
    b.attr_last_move(MoveLineEdit::Retarget(p1b()));
    assert_eq!(last(&b), "|move|p1a: Wugtrio|Protect|p1b: Dondozo|[still]");
}

#[test]
fn short_lines_are_padded_like_js_arrays() {
    // Choice-lock / Gorilla Tactics fail paths print a bare `|move|p|Name` (data/conditions.ts:342).
    let mut b = standard_active();
    let bare = |b: &mut Battle<TextLog>| {
        b.add_move(LogEntry::new("move", &[LogArg::Mon(p1a()), LogArg::Text("Protect")], &[]));
    };
    bare(&mut b);
    assert_eq!(last(&b), "|move|p1a: Wugtrio|Protect");
    b.attr_last_move(MoveLineEdit::Still);
    assert_eq!(last(&b), "|move|p1a: Wugtrio|Protect||[still]");
    bare(&mut b);
    b.attr_last_move(MoveLineEdit::Retarget(p2a()));
    assert_eq!(last(&b), "|move|p1a: Wugtrio|Protect|p2a: Klefki");
}

#[test]
fn edits_land_on_the_move_line_not_the_last_line() {
    let mut b = standard_active();
    add_move(&mut b, p2a(), "Rock Slide", p1a(), &[]);
    add(&mut b, "-supereffective", &[LogArg::Mon(p1a())], &[]);
    add_split(&mut b, "-damage", &[LogArg::Mon(p1a()), LogArg::Health(p1a())], &[], 0, false);
    add(&mut b, "-crit", &[LogArg::Mon(p1b())], &[]);
    b.attr_last_move(MoveLineEdit::Tag(LogTag::Value(
        "spread",
        LogArg::Spread { mons: [p1a(), p1b(), MonId::NONE, MonId::NONE], len: 2 },
    )));
    let l = lines(&b);
    assert_eq!(l[0], "|move|p2a: Klefki|Rock Slide|p1a: Wugtrio|[spread] p1a,p1b");
    assert_eq!(l.len(), 6, "an edit never appends: {l:?}");
    assert_eq!(l[5], "|-crit|p1b: Dondozo");
}

#[test]
fn nested_move_steals_the_pointer() {
    // One lastMoveLine, not a stack: the outer move's later [spread] lands on the inner line
    // (Magic Bounce / Dancer, 05-protocol.md section 2.2).
    let mut b = standard_active();
    add_move(&mut b, p1a(), "String Shot", p2a(), &[]);
    add(&mut b, "-ability", &[LogArg::Mon(p2a()), LogArg::Text("Magic Bounce")], &[]);
    add_move(&mut b, p2a(), "String Shot", p1a(), &[LogTag::From(fx(EffectKind::Ability, "Magic Bounce"))]);
    b.attr_last_move(MoveLineEdit::Tag(LogTag::Value(
        "spread",
        LogArg::Spread { mons: [p1a(), p1b(), MonId::NONE, MonId::NONE], len: 2 },
    )));
    let l = lines(&b);
    assert_eq!(l[0], "|move|p1a: Wugtrio|String Shot|p2a: Klefki");
    assert_eq!(
        l[2],
        "|move|p2a: Klefki|String Shot|p1a: Wugtrio|[from] ability: Magic Bounce|[spread] p1a,p1b"
    );
}

#[test]
fn edit_without_a_move_line_is_a_no_op() {
    let mut b = standard_active();
    for edit in [
        MoveLineEdit::Still,
        MoveLineEdit::Miss,
        MoveLineEdit::NoTarget,
        MoveLineEdit::Spread(0),
        MoveLineEdit::Retarget(p1a()),
        MoveLineEdit::Tag(LogTag::Bare("x")),
    ] {
        b.attr_last_move(edit);
    }
    assert!(b.log.entries.is_empty());
    add(&mut b, "-fail", &[LogArg::Mon(p1a())], &[]);
    b.attr_last_move(MoveLineEdit::Still);
    assert_eq!(lines(&b), ["|-fail|p1a: Wugtrio"]);
    assert_eq!(b.scratch.unsent_lines, 5, "4 pre-start lines + 1; edits never count");
}

#[test]
fn anim_line_is_deleted_by_still_and_the_pointer_is_cleared() {
    let mut b = standard_active();
    add(&mut b, "-prepare", &[LogArg::Mon(p1a()), LogArg::Text("Solar Beam")], &[]);
    b.add_move(LogEntry::new(
        "-anim",
        &[LogArg::Mon(p1a()), LogArg::Text("Solar Beam"), LogArg::Mon(p2a())],
        &[],
    ));
    add(&mut b, "-fail", &[LogArg::Mon(p1a())], &[]);
    b.attr_last_move(MoveLineEdit::Still);
    assert_eq!(lines(&b), ["|-prepare|p1a: Wugtrio|Solar Beam", "|-fail|p1a: Wugtrio"]);
    assert_eq!(b.log.last_move_line, None);
    // Nothing is left to edit.
    b.attr_last_move(MoveLineEdit::Miss);
    assert_eq!(b.log.entries.len(), 2);
}

#[test]
fn anim_line_takes_other_attrs() {
    let mut b = standard_active();
    b.add_move(LogEntry::new(
        "-anim",
        &[LogArg::Mon(p2b()), LogArg::Text("Meteor Beam"), LogArg::Mon(p1a())],
        &[],
    ));
    b.attr_last_move(MoveLineEdit::Miss);
    assert_eq!(last(&b), "|-anim|p2b: Reshiram|Meteor Beam|p1a: Wugtrio|[miss]");
    b.add_move(LogEntry::new(
        "-anim",
        &[LogArg::Mon(p2b()), LogArg::Text("Solar Beam"), LogArg::Mon(mon(0, 2))],
        &[],
    ));
    b.attr_last_move(MoveLineEdit::NoTarget);
    assert_eq!(last(&b), "|-anim|p2b: Reshiram|Solar Beam|p1: Gholdengo|[notarget]");
}

#[test]
fn drained_lines_can_still_be_edited_and_are_not_resent() {
    let mut b = standard_active();
    add_move(&mut b, p1a(), "Icy Wind", p2a(), &[]);
    let mut out = Vec::new();
    b.drain_log(&mut out);
    assert_eq!(out, ["|move|p1a: Wugtrio|Icy Wind|p2a: Klefki"]);
    b.attr_last_move(MoveLineEdit::Miss);
    assert_eq!(b.log.entries[0], "|move|p1a: Wugtrio|Icy Wind|p2a: Klefki|[miss]");
    out.clear();
    b.drain_log(&mut out);
    assert!(out.is_empty(), "history is kept, only new entries are drained");
    add(&mut b, "-miss", &[LogArg::Mon(p1a()), LogArg::Mon(p2a())], &[]);
    b.drain_log(&mut out);
    assert_eq!(out, ["|-miss|p1a: Wugtrio|p2a: Klefki"]);
}

#[test]
fn illusion_disguise_is_what_the_move_line_shows() {
    let mut b = standard();
    // Zoroark (p1 party 4) active in slot a, disguised as Porygon2 (party 5).
    activate(&mut b, mon(0, 4), 0);
    b.state.pokemon[mon(0, 4).0 as usize].illusion = mon(0, 5);
    activate(&mut b, mon(1, 0), 0);
    add_move(&mut b, mon(0, 4), "Nasty Plot", mon(0, 4), &[]);
    assert_eq!(last(&b), "|move|p1a: Porygon2|Nasty Plot|p1a: Porygon2");
    b.attr_last_move(MoveLineEdit::Retarget(mon(1, 0)));
    assert_eq!(last(&b), "|move|p1a: Porygon2|Nasty Plot|p2a: Klefki");
}
