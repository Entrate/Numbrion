//! OWNER T: formatting only at the enabled protocol/request output boundary.
//!
//! Every function appends to a caller-owned `String` and draws nothing from the PRNG. These
//! functions are reached only through an enabled sink (`LogSink::ENABLED`), never in the
//! battle loop of a `NoLog` instantiation.
//!
//! `Array.prototype.join` semantics (battle.ts:3091-3113) are the model: `null`/`undefined`
//! keep their field but print as the empty string, numbers print as JS decimals, `Pokemon`
//! and `Side` objects print through their `toString`, effect objects through `.name` unless
//! the call site wrote `.fullname`, and the two function parts (`getHealth`,
//! `getFullDetails`) are evaluated by the caller once per view (secret/shared).
use super::*;
use crate::{
    dex,
    state::{
        Status, mon_flags,
        scratch::{EffectRef, SyntheticEffect},
    },
    teams::SetDef,
};
use core::fmt::Write as _;

/// Per-call context threaded through nested `Parts`.
#[derive(Clone, Copy)]
struct Ctx<'a> {
    view: LogView<'a>,
    /// Secret (omniscient / owner) view: exact HP. Otherwise the public HP Percentage Mod view.
    secret: bool,
    /// `replace` prints `getUpdatedDetails()` (no Illusion substitution, no tera suffix,
    /// data/abilities.ts:2082); every other `Details` caller prints `(illusion || this).details`
    /// plus the tera suffix (`detailschange`, pokemon.ts:1445-1446).
    plain_details: bool,
}

fn set_of<'a>(view: LogView<'a>, mon: MonId) -> &'a SetDef {
    &view.teams.sides[mon.side().0 as usize].sets[mon.0 as usize % 6]
}

fn push_int(out: &mut String, n: impl core::fmt::Display) {
    let _ = write!(out, "{n}");
}

/// `side.id`: `p1` / `p2` (side.ts:353 uses it in `Side.toString`).
fn write_side_id(out: &mut String, side: SideId) {
    out.push('p');
    push_int(out, side.0 as u32 + 1);
}

/// `Pokemon.getSlot()` (pokemon.ts:520): side id plus `abcdef[position]`. Two-player format,
/// so `positionOffset` is always zero.
fn write_slot(out: &mut String, view: LogView<'_>, mon: MonId) {
    let p = &view.state.pokemon[mon.0 as usize];
    write_side_id(out, mon.side());
    out.push((b'a' + p.position) as char);
}

/// Ports pokemon.ts:531-552. PRNG: none. Illusion and active/bench ident semantics.
///
/// `Pokemon.toString`: `fullname = (illusion || this).fullname`; active mons print
/// `getSlot() + fullname.slice(2)` (`p1a: Name`), everything else `fullname` (`p1: Name`).
/// The ident name is `set.name`, which is never a forme name.
pub fn write_ident(out: &mut String, view: LogView<'_>, mon: MonId) {
    let p = &view.state.pokemon[mon.0 as usize];
    let named = if p.illusion != MonId::NONE {
        p.illusion
    } else {
        mon
    };
    if p.flags & mon_flags::ACTIVE != 0 {
        write_slot(out, view, mon);
    } else {
        write_side_id(out, named.side());
    }
    out.push_str(": ");
    out.push_str(&set_of(view, named).name);
}

/// `Pokemon.fullname` (pokemon.ts:335): `pN: Name` with the mon's own name, whether or not it is
/// active and whatever disguise is up. Request JSON writes this as `ident` (pokemon.ts:1155).
pub fn write_fullname(out: &mut String, view: LogView<'_>, mon: MonId) {
    write_side_id(out, mon.side());
    out.push_str(": ");
    out.push_str(&set_of(view, mon).name);
}

/// The stored `Pokemon.details` field: `getUpdatedDetails()` evaluated with `species == baseSpecies`
/// (pokemon.ts:377,1443): no Illusion substitution and no tera suffix. Request JSON writes this
/// as `details` (pokemon.ts:1156).
pub fn write_stored_details(out: &mut String, view: LogView<'_>, mon: MonId) {
    push_updated_details(
        out,
        view,
        mon,
        view.state.pokemon[mon.0 as usize].base_species,
    );
}

pub(crate) fn status_name(status: Status) -> &'static str {
    match status {
        Status::None => "",
        Status::Burn => "brn",
        Status::Paralysis => "par",
        Status::Sleep => "slp",
        Status::Freeze => "frz",
        Status::Poison => "psn",
        Status::Toxic => "tox",
        Status::Fainted => "fnt",
    }
}

/// `TypeInfo.toString` (dex-data.ts:287): the type name; `TypeId::NONE` is the empty field.
pub(crate) fn type_name(t: TypeId) -> &'static str {
    if t == TypeId::NONE {
        ""
    } else {
        dex::TYPE_NAMES[t.0 as usize - 1]
    }
}

/// Ports sim/pokemon.ts:2060-2107; data/rulesets.ts:1353-1360 (HP Percentage Mod).
/// PRNG: none. Secret is exact HP; shared follows the format's HP percentage rule.
///
/// Gen 9 always takes the `reportPercentages || gen >= 7` branch (pokemon.ts:2075-2082):
/// `ceil(100 * hp / maxhp)`, clamped to 99 while `hp < maxhp`. A fainted mon is `0 fnt` in both
/// views, without a status suffix. The status id is appended to both views (`tox` stays `tox`).
pub fn write_health(out: &mut String, view: LogView<'_>, mon: MonId, secret: bool) {
    let p = &view.state.pokemon[mon.0 as usize];
    if p.hp == 0 {
        out.push_str("0 fnt");
        return;
    }
    if secret {
        push_int(out, p.hp);
        out.push('/');
        push_int(out, p.max_hp);
    } else {
        // Exact in floating point too: a non-integer quotient is at least 1/maxhp from an integer.
        let hp = p.hp as u32;
        let max = (p.max_hp as u32).max(1);
        let mut pct = (100 * hp).div_ceil(max);
        if pct == 100 && hp < max {
            pct = 99;
        }
        push_int(out, pct);
        out.push_str("/100");
    }
    if p.status != Status::None {
        out.push(' ');
        out.push_str(status_name(p.status));
    }
}

/// `getUpdatedDetails(level)` body (pokemon.ts:536-540): species name (the two forme-name
/// exceptions print their base species), `, L<level>` unless 100, `, <gender>` unless genderless,
/// `, shiny`.
fn push_updated_details(out: &mut String, view: LogView<'_>, mon: MonId, species: EffectId) {
    let s = dex::species(species);
    let set = set_of(view, mon);
    let name = if s.name == "Greninja-Bond" || s.name == "Rockruff-Dusk" {
        s.base_species_name
    } else {
        s.name
    };
    out.push_str(name);
    if set.level != 100 {
        out.push_str(", L");
        push_int(out, set.level);
    }
    let gender = set.gender.protocol();
    if !gender.is_empty() {
        out.push_str(", ");
        out.push_str(gender);
    }
    if set.shiny {
        out.push_str(", shiny");
    }
}

/// Ports sim/pokemon.ts:524-529,544-552. PRNG: none. Full details respects Illusion.
///
/// `full == false`: `getUpdatedDetails()` of `mon` itself (current species, own level).
/// `full == true`: the details half of `getFullDetails` / the `detailschange` argument: the
/// Illusion target's details when a disguise is up (Illusion Level Mod prints the target's own
/// level), otherwise the stored `details` (`baseSpecies` form, pokemon.ts:1443), followed by
/// `, tera:<Type>` while the real mon is terastallized.
pub fn write_details(out: &mut String, view: LogView<'_>, mon: MonId, full: bool) {
    let p = &view.state.pokemon[mon.0 as usize];
    if !full {
        push_updated_details(out, view, mon, p.species);
        return;
    }
    if p.illusion != MonId::NONE {
        let target = &view.state.pokemon[p.illusion.0 as usize];
        push_updated_details(out, view, p.illusion, target.species);
    } else {
        push_updated_details(out, view, mon, p.base_species);
    }
    if p.terastallized != TypeId::NONE {
        out.push_str(", tera:");
        out.push_str(type_name(p.terastallized));
    }
}

fn synthetic_name(s: SyntheticEffect) -> &'static str {
    match s {
        // `this.format`: the Format effect (dex-formats.ts), name == fullname.
        SyntheticEffect::Format => "[Gen 9] Random Doubles Battle",
        // Plain `{id: ...}` objects in the TS source carry no name; the id is the best label.
        SyntheticEffect::Confused => "confused",
        SyntheticEffect::StruggleRecoil => "strugglerecoil",
        SyntheticEffect::Recharge => "recharge",
        SyntheticEffect::Fainted => "fainted",
        SyntheticEffect::MindBlownRecoil => "mindblown",
        // dex.conditions.getByID('recoil') (dex-conditions.ts:699).
        SyntheticEffect::Recoil => "Recoil",
    }
}

fn dex_name(id: EffectId) -> &'static str {
    dex::effect(id).name
}

/// Ports dex-data.ts:129-143 and dex-moves.ts:477. PRNG: none. Name versus fullname.
///
/// `BasicEffect.toString` is `.name`. `.fullname` is `move: X`, `ability: X`, `item: X`,
/// `pokemon: X` for those dex kinds and the bare name for everything else (conditions, rules,
/// formats). The `*Condition` views are condition objects built from a move/ability/item/species
/// record (dex-conditions.ts:690-700): bare name in both forms.
pub fn write_effect(out: &mut String, view: LogView<'_>, effect: EffectRef, fullname: bool) {
    match effect {
        EffectRef::None => {}
        EffectRef::Dex(id) => {
            if fullname {
                out.push_str(match id.kind() {
                    Some(EffectKind::Move) => "move: ",
                    Some(EffectKind::Ability) => "ability: ",
                    Some(EffectKind::Item) => "item: ",
                    Some(EffectKind::Species) => "pokemon: ",
                    _ => "",
                });
            }
            out.push_str(dex_name(id));
        }
        EffectRef::ActiveMove(handle) => {
            let m = view.moves[handle as usize]
                .as_ref()
                .expect("log effect names a released active move");
            if fullname {
                out.push_str("move: ");
            }
            out.push_str(dex_name(m.id));
        }
        EffectRef::SpeciesCondition(id)
        | EffectRef::MoveCondition(id)
        | EffectRef::AbilityCondition(id)
        | EffectRef::ItemCondition(id) => out.push_str(dex_name(id)),
        EffectRef::Synthetic(s) => out.push_str(synthetic_name(s)),
        EffectRef::Synchronize(_) => out.push_str("synchronize"),
    }
}

/// ECMAScript `Number::toString(10)` for a finite or special double (used by `join`).
fn write_js_number(out: &mut String, n: f64) {
    if n.is_nan() {
        out.push_str("NaN");
        return;
    }
    if n == 0.0 {
        // `-0` prints `0`.
        out.push('0');
        return;
    }
    if n.is_infinite() {
        out.push_str(if n < 0.0 { "-Infinity" } else { "Infinity" });
        return;
    }
    if n < 0.0 {
        out.push('-');
    }
    // Shortest round-trip digits and exponent from Rust's `{:e}`: d.ddde<exp>.
    let sci = format!("{:e}", n.abs());
    let (mantissa, exp) = sci.split_once('e').expect("scientific notation");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    // value = 0.<digits> * 10^point
    let point = exp.parse::<i32>().expect("exponent") + 1;
    if k <= point && point <= 21 {
        out.push_str(&digits);
        for _ in 0..point - k {
            out.push('0');
        }
    } else if 0 < point && point <= 21 {
        out.push_str(&digits[..point as usize]);
        out.push('.');
        out.push_str(&digits[point as usize..]);
    } else if -6 < point && point <= 0 {
        out.push_str("0.");
        for _ in 0..-point {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        let e = point - 1;
        out.push(if e < 0 { '-' } else { '+' });
        push_int(out, e.abs());
    }
}

fn write_arg_in(out: &mut String, cx: Ctx<'_>, arg: LogArg<'_>) {
    let view = cx.view;
    match arg {
        LogArg::Text(s) => out.push_str(s),
        LogArg::Parts(parts) => {
            for part in parts {
                write_arg_in(out, cx, *part);
            }
        }
        LogArg::Empty => {}
        LogArg::Bool(b) => out.push_str(if b { "true" } else { "false" }),
        LogArg::Mon(mon) => {
            // A null/undefined Pokemon keeps its field and prints empty.
            if mon != MonId::NONE {
                write_ident(out, view, mon);
            }
        }
        LogArg::Side(side) => {
            // Side.toString (side.ts:353): `p2: Bob`.
            write_side_id(out, side);
            out.push_str(": ");
            out.push_str(&view.player_names[side.0 as usize]);
        }
        LogArg::SideId(side) => write_side_id(out, side),
        LogArg::PlayerName(side) => out.push_str(&view.player_names[side.0 as usize]),
        LogArg::Effect(effect) => write_effect(out, view, effect, false),
        LogArg::EffectFullName(effect) => write_effect(out, view, effect, true),
        LogArg::Type(t) => out.push_str(type_name(t)),
        LogArg::Status(s) => out.push_str(status_name(s)),
        LogArg::Number(n) => push_int(out, n),
        LogArg::Decimal(n) => write_js_number(out, n),
        LogArg::Health(mon) => write_health(out, view, mon, cx.secret),
        LogArg::Details(mon) => write_details(out, view, mon, !cx.plain_details),
        LogArg::FullDetails(mon) => {
            // getFullDetails (pokemon.ts:544): ONE part `details|health` (hence the 4-field switch line).
            write_details(out, view, mon, true);
            out.push('|');
            write_health(out, view, mon, cx.secret);
        }
        LogArg::Species(id) => out.push_str(dex_name(id)),
        LogArg::Spread { mons, len } => {
            for (i, mon) in mons[..len as usize].iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_slot(out, view, *mon);
            }
        }
    }
}

/// Ports battle.ts:3091-3113. PRNG: none. JS join null/undefined preserve empty fields.
///
/// `secret` selects the view of the two HP-dependent arguments (`Health`, `FullDetails`):
/// exact HP when true (secret line and omniscient log), HP Percentage Mod when false.
pub fn write_arg(out: &mut String, view: LogView<'_>, arg: LogArg<'_>, secret: bool) {
    write_arg_in(
        out,
        Ctx {
            view,
            secret,
            plain_details: false,
        },
        arg,
    );
}

fn write_tag_in(out: &mut String, cx: Ctx<'_>, tag: LogTag<'_>) {
    match tag {
        // `'[from] ' + effect.fullname`
        LogTag::From(effect) => {
            out.push_str("[from] ");
            write_effect(out, cx.view, effect, true);
        }
        LogTag::Of(mon) => {
            out.push_str("[of] ");
            if mon != MonId::NONE {
                write_ident(out, cx.view, mon);
            }
        }
        LogTag::Bare(name) => {
            out.push('[');
            out.push_str(name);
            out.push(']');
        }
        // `[name] <arg>`, e.g. `[wisher] Vaporeon`, `[anim] Tera Blast Ground`, `[spread] ` (empty).
        LogTag::Value(name, arg) => {
            out.push('[');
            out.push_str(name);
            out.push_str("] ");
            write_arg_in(out, cx, arg);
        }
        LogTag::Text(s) => out.push_str(s),
    }
}

/// One tag exactly as `attrLastMove` receives it (no leading `|`).
pub(crate) fn write_tag(out: &mut String, view: LogView<'_>, tag: LogTag<'_>) {
    write_tag_in(
        out,
        Ctx {
            view,
            secret: true,
            plain_details: false,
        },
        tag,
    );
}

/// True if any argument (including nested `Parts`) is a function part (`getHealth` /
/// `getFullDetails`), which in Showdown forces the `|split|` triple (battle.ts:3098-3112).
fn is_split_arg(arg: &LogArg<'_>) -> bool {
    match arg {
        LogArg::Health(_) | LogArg::FullDetails(_) => true,
        LogArg::Parts(parts) => parts.iter().any(is_split_arg),
        _ => false,
    }
}

/// Builds `'|' + [command, ...args, ...tags].join('|')` for one view of `entry`.
pub(crate) fn write_line(out: &mut String, view: LogView<'_>, entry: &LogEntry<'_>, secret: bool) {
    assert!(
        entry.split_side.is_some() || !entry.args.iter().any(is_split_arg),
        "log entry `{}` has an HP-dependent argument but no split side",
        entry.command
    );
    let cx = Ctx {
        view,
        secret,
        // `replace` announces the real mon with `getUpdatedDetails()`.
        plain_details: entry.command == "replace",
    };
    out.push('|');
    out.push_str(entry.command);
    if entry.command == "t:" && entry.args.is_empty() && entry.tags.is_empty() {
        // The Battle constructor/turnLoop add `t:` with a timestamp argument; fixtures hold `|t:|`.
        out.push('|');
        return;
    }
    for arg in entry.args {
        out.push('|');
        write_arg_in(out, cx, *arg);
    }
    for tag in entry.tags {
        out.push('|');
        write_tag_in(out, cx, *tag);
    }
}
