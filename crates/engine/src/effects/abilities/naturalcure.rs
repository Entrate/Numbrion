//! Ports data/abilities.ts:2812 (Natural Cure). No direct PRNG draws; `clearStatus` runs
//! SetStatus events whose listeners may speed-sort ties.
//!
//! State: the only mutable data is `Pokemon::show_cure` (`showCure?: boolean`, pokemon.ts:94,382),
//! kept in the Pokemon snapshot as `ResultFlag::{Undefined, False, True}` (never `Null`); the
//! ability's effect state is not used. `onCheckShow` is reached by `singleEvent('CheckShow',
//! naturalcure, null, action.pokemon)` (battle.ts:2768), so its hook state is a temporary cell.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{ResultFlag, Status},
};
pub const ID: EffectId = dex::ABILITY_NATURALCURE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_NATURALCURE_ONCHECKSHOW,
    dex::HOOK_ABILITY_NATURALCURE_ONSWITCHOUT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_NATURALCURE_ONCHECKSHOW => on_check_show(b, cx),
        dex::HOOK_ABILITY_NATURALCURE_ONSWITCHOUT => on_switch_out(b, cx),
        _ => panic!("unexpected naturalcure hook"),
    }
}

// data/abilities.ts:2813-2873 onCheckShow(pokemon). PRNG: none. Returns undefined on every path.
fn on_check_show<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // if (pokemon.side.active.length === 1) return;  -- doubles: side.active.length is always 2.
    // if (pokemon.showCure === true || pokemon.showCure === false) return;
    if matches!(
        b.state.pokemon[pokemon.0 as usize].show_cure,
        ResultFlag::True | ResultFlag::False
    ) {
        return Relay::Undefined;
    }
    let side = pokemon.side();
    let active = b.state.sides[side.0 as usize].active;
    let mut cure_list = [MonId::NONE; 2];
    let mut cure_len = 0usize;
    let mut no_cure_count = 0u32;
    for cur in active {
        // if (!curPoke?.status) continue;   (an empty slot has no status either)
        if cur == MonId::NONE {
            continue;
        }
        let mon = &b.state.pokemon[cur.0 as usize];
        if mon.status == Status::None {
            continue;
        }
        // if (curPoke.showCure) continue;   (showCure is true here: false/undefined are falsy)
        if mon.show_cure == ResultFlag::True {
            continue;
        }
        // const species = curPoke.species; Object.values(species.abilities).includes('Natural Cure')
        let species = dex::species(mon.species);
        if !species.abilities.iter().any(|slot| slot.id == ID) {
            continue;
        }
        // if (!species.abilities['1'] && !species.abilities['H']) continue;  (slots 0,1,H,S)
        if species.abilities[1].key.is_empty() && species.abilities[2].key.is_empty() {
            continue;
        }
        // if (curPoke !== pokemon && !this.queue.willSwitch(curPoke)) continue;
        if cur != pokemon && b.queue_will_switch(cur).is_none() {
            continue;
        }
        if b.has_ability(cur, &[ID]) {
            // cureList.push(curPoke);
            cure_list[cure_len] = cur;
            cure_len += 1;
        } else {
            // noCureCount++;
            no_cure_count += 1;
        }
    }
    if cure_len == 0 || no_cure_count == 0 {
        // It's possible to know what pokemon were cured: pkmn.showCure = true.
        for &cured in &cure_list[..cure_len] {
            b.state.pokemon[cured.0 as usize].show_cure = ResultFlag::True;
        }
    } else {
        // this.add('-message', `(${n} of ${pokemon.side.name}'s pokemon ${n === 1 ? "was" : "were"}
        // cured by Natural Cure.)`);
        b.add(LogEntry::new(
            "-message",
            &[LogArg::Parts(&[
                LogArg::Text("("),
                LogArg::Number(cure_len as i32),
                LogArg::Text(" of "),
                LogArg::PlayerName(side),
                LogArg::Text("'s pokemon "),
                LogArg::Text(if cure_len == 1 { "was" } else { "were" }),
                LogArg::Text(" cured by Natural Cure.)"),
            ])],
            &[],
        ));
        // pkmn.showCure = false;
        for &cured in &cure_list[..cure_len] {
            b.state.pokemon[cured.0 as usize].show_cure = ResultFlag::False;
        }
    }
    Relay::Undefined
}

// data/abilities.ts:2876-2890 onSwitchOut(pokemon). PRNG: none directly.
fn on_switch_out<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    let index = pokemon.0 as usize;
    // if (!pokemon.status) return;
    let status = b.state.pokemon[index].status;
    if status == Status::None {
        return Relay::Undefined;
    }
    // if (pokemon.showCure === undefined) pokemon.showCure = true;
    if b.state.pokemon[index].show_cure == ResultFlag::Undefined {
        b.state.pokemon[index].show_cure = ResultFlag::True;
    }
    // if (pokemon.showCure) this.add('-curestatus', pokemon, pokemon.status,
    //   '[from] ability: Natural Cure', '[silent]');
    if b.state.pokemon[index].show_cure == ResultFlag::True {
        b.add(LogEntry::new(
            "-curestatus",
            &[LogArg::Mon(pokemon), LogArg::Status(status)],
            &[
                LogTag::From(crate::event::EffectRef::Dex(ID)),
                LogTag::Bare("silent"),
            ],
        ));
    }
    // pokemon.clearStatus();
    b.clear_status(pokemon);
    // only reset .showCure if it's false: if (!pokemon.showCure) pokemon.showCure = undefined;
    if b.state.pokemon[index].show_cure != ResultFlag::True {
        b.state.pokemon[index].show_cure = ResultFlag::Undefined;
    }
    Relay::Undefined
}
