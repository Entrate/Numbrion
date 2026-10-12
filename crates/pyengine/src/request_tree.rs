//! A side's request as the `encoder::json::Json` tree that parsing `Game::request_json` gives, built straight from
//! the engine's request data: no text is formatted or parsed. `SearchGrid` encodes its leaves from it.
//!
//! It mirrors `Battle::write_request_json` (engine `sim/choices/requests.rs`) member for member, in key order, so
//! the tree equals `Json::parse(&request_json(side))` exactly (strings unescaped, numbers as f64). The tests
//! (`tests/search_grid.rs`) compare the two at every boundary they reach, including rejected choices that
//! update a request; a change to the engine's request JSON must be repeated here.

use encoder::json::Json;
use engine::{
    Battle, dex,
    ids::{EffectId, MonId, SideId, TypeId},
    log::LogSink,
    sim::choices::{TargetKind, move_presence as mp, request_flags as rf},
    state::{
        Status,
        choices::{ActiveRequest, RequestKind, RequestMove},
    },
};
use std::borrow::Cow;

type Members<'a> = Vec<(Cow<'a, str>, Json<'a>)>;

fn text(s: &str) -> Json<'_> {
    Json::Str(Cow::Borrowed(s))
}

fn key(k: &'static str) -> Cow<'static, str> {
    Cow::Borrowed(k)
}

/// The `id` string of an effect, or "" for none.
fn id_key(id: EffectId) -> &'static str {
    if id == EffectId::NONE { "" } else { dex::effect(id).key }
}

/// The name of a type, "" for none.
fn type_name(t: TypeId) -> &'static str {
    if t == TypeId::NONE { "" } else { dex::TYPE_NAMES[t.0 as usize - 1] }
}

fn status_name(s: Status) -> &'static str {
    match s {
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

/// `side`'s request tree; None when it has no request (after the end).
pub fn request_tree<L: LogSink>(b: &Battle<L>, side: usize) -> Option<Json<'_>> {
    let req = &b.state.requests[side];
    let mut top: Members<'_> = Vec::with_capacity(4);
    match req.kind {
        RequestKind::None => return None,
        RequestKind::Wait => top.push((key("wait"), Json::Bool(true))),
        RequestKind::Move => top.push((key("active"), Json::Arr(req.active.iter().map(active_json).collect()))),
        RequestKind::Switch => {
            let mask = (0..2).map(|k| Json::Bool(req.force_switch_mask & (1 << k) != 0)).collect();
            top.push((key("forceSwitch"), Json::Arr(mask)));
        }
    }
    top.push((key("side"), side_json(b, SideId(side as u8))));
    if req.no_cancel {
        top.push((key("noCancel"), Json::Bool(true)));
    }
    if req.update {
        top.push((key("update"), Json::Bool(true)));
    }
    Some(Json::Obj(top))
}

/// `{"name":..,"id":..,"pokemon":[..]}` (Side.getRequestData).
fn side_json<L: LogSink>(b: &Battle<L>, side: SideId) -> Json<'_> {
    let data = b.get_side_request_data(side);
    let pokemon = data.pokemon[..data.pokemon_count as usize].iter().map(|&m| pokemon_json(b, m)).collect();
    Json::Obj(vec![
        (key("name"), text(b.player_name(side))),
        (key("id"), text(["p1", "p2"][side.0 as usize])),
        (key("pokemon"), Json::Arr(pokemon)),
    ])
}

/// One `Pokemon.getSwitchRequestData` entry.
fn pokemon_json<L: LogSink>(b: &Battle<L>, mon: MonId) -> Json<'_> {
    let d = b.get_switch_request_data(mon);
    let p = &b.state.pokemon[mon.0 as usize];
    let set = &b.teams().sides[mon.side().0 as usize].sets[mon.0 as usize % 6];
    let condition = if p.hp == 0 {
        Cow::Borrowed("0 fnt")
    } else if p.status == Status::None {
        Cow::Owned(format!("{}/{}", p.hp, p.max_hp))
    } else {
        Cow::Owned(format!("{}/{} {}", p.hp, p.max_hp, status_name(p.status)))
    };
    let stats = ["atk", "def", "spa", "spd", "spe"].into_iter().zip(d.stats).map(|(k, v)| (key(k), Json::Num(f64::from(v))));
    let moves = d.moves[..d.move_count as usize].iter().map(|&m| text(id_key(m))).collect();
    Json::Obj(vec![
        (key("ident"), Json::Str(Cow::Owned(format!("p{}: {}", mon.side().0 + 1, set.name)))),
        (key("details"), Json::Str(Cow::Owned(b.details(mon)))),
        (key("condition"), Json::Str(condition)),
        (key("active"), Json::Bool(d.active)),
        (key("stats"), Json::Obj(stats.collect())),
        (key("moves"), Json::Arr(moves)),
        (key("baseAbility"), text(id_key(d.base_ability))),
        (key("item"), text(id_key(d.item))),
        (key("pokeball"), text(&set.pokeball)),
        (key("ability"), text(id_key(d.ability))),
        (key("commanding"), Json::Bool(d.commanding)),
        (key("reviving"), Json::Bool(d.reviving)),
        (key("teraType"), text(type_name(d.tera_type))),
        (key("terastallized"), text(type_name(d.terastallized))),
    ])
}

/// One `PokemonMoveRequestData` (an `active` entry).
fn active_json(a: &ActiveRequest) -> Json<'static> {
    let mut m: Members<'static> = Vec::with_capacity(4);
    m.push((key("moves"), Json::Arr(a.moves[..a.move_count as usize].iter().map(move_json).collect())));
    if a.flags & rf::MAYBE_DISABLED != 0 {
        m.push((key("maybeDisabled"), Json::Bool(true)));
    }
    if a.flags & rf::MAYBE_LOCKED != 0 {
        m.push((key("maybeLocked"), Json::Bool(true)));
    }
    if a.flags & rf::TRAPPED != 0 && a.flags & rf::TRAPPED_LATE == 0 {
        m.push((key("trapped"), Json::Bool(true)));
    }
    if a.flags & rf::MAYBE_TRAPPED != 0 {
        m.push((key("maybeTrapped"), Json::Bool(true)));
    }
    if a.can_tera != TypeId::NONE {
        m.push((key("canTerastallize"), text(type_name(a.can_tera))));
    }
    if a.flags & rf::TRAPPED != 0 && a.flags & rf::TRAPPED_LATE != 0 {
        m.push((key("trapped"), Json::Bool(true)));
    }
    Json::Obj(m)
}

fn move_json(r: &RequestMove) -> Json<'static> {
    let mut m: Members<'static> = Vec::with_capacity(6);
    if r.presence & mp::RECHARGE != 0 {
        m.push((key("move"), text("Recharge")));
        m.push((key("id"), text("recharge")));
    } else {
        m.push((key("move"), text(dex::move_data(r.id).name)));
        m.push((key("id"), text(dex::effect(r.id).key)));
    }
    if r.presence & mp::PP != 0 {
        m.push((key("pp"), Json::Num(f64::from(r.pp))));
    }
    if r.presence & mp::MAX_PP != 0 {
        m.push((key("maxpp"), Json::Num(f64::from(r.max_pp))));
    }
    if r.presence & mp::TARGET != 0 {
        let target = if r.presence & mp::ADJACENT_FOE != 0 { TargetKind::AdjacentFoe } else { TargetKind::from(r.target) };
        m.push((key("target"), text(target.name())));
    }
    if r.presence & mp::DISABLED != 0 {
        m.push((key("disabled"), Json::Bool(r.disabled != 0)));
    }
    if r.presence & mp::DISABLED_SOURCE != 0 {
        let source = if r.disabled_source == EffectId::NONE { "" } else { dex::effect(r.disabled_source).name };
        m.push((key("disabledSource"), text(source)));
    }
    Json::Obj(m)
}
