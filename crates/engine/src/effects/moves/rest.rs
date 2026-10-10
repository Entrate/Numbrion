//! Rest (moves:rest). Ports data/moves.ts:14957-14994.
//!
//! Payload: none. `onHit` writes into the *Sleep* condition's cell (`target.statusState`),
//! using the layout documented in `conditions/slp.rs`: word0 = startTime (present bit 8),
//! word1 = time (present bit 9, signed).
use crate::{
    Battle,
    actions::{Attribution, HealEffect},
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg},
    },
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{CellId, Status, present},
};

pub const ID: EffectId = dex::MOVE_REST;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_REST_ONTRY, dex::HOOK_MOVE_REST_ONHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

/// Sleep status-cell custom bits (conditions/slp.rs): startTime / time.
const SLP_START_TIME: u32 = 1 << present::CUSTOM_START;
const SLP_TIME: u32 = 1 << (present::CUSTOM_START + 1);

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_REST_ONTRY => on_try(b, cx),
        dex::HOOK_MOVE_REST_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected Rest function site"),
    }
}

/// data/moves.ts:14966-14982, `onTry(source)` (singleEvent('Try', move, null, pokemon, target,
/// move): arg0 is the user). PRNG: none.
/// Returns false (already asleep / Comatose; no log line), null after a logged `-fail`, else
/// undefined. The Insomnia and Vital Spirit checks are separate so each prints its own message.
fn on_try<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let source = mon_arg(b, cx, 0);
    if b.state.pokemon[source.0 as usize].status == Status::Sleep
        || b.has_ability(source, &[dex::ABILITY_COMATOSE])
    {
        return Relay::Bool(false);
    }
    let mon = &b.state.pokemon[source.0 as usize];
    if mon.hp == mon.max_hp {
        b.add(LogEntry::new(
            "-fail",
            &[LogArg::Mon(source), LogArg::Text("heal")],
            &[],
        ));
        return Relay::Null;
    }
    // `-fail|<source>|[from] ability: Insomnia|[of] <source>` (then Vital Spirit).
    for ability in [dex::ABILITY_INSOMNIA, dex::ABILITY_VITALSPIRIT] {
        if b.has_ability(source, &[ability]) {
            b.add(LogEntry::new(
                "-fail",
                &[LogArg::Mon(source)],
                &[LogTag::From(EffectRef::Dex(ability)), LogTag::Of(source)],
            ));
            return Relay::Null;
        }
    }
    Relay::Undefined
}

/// data/moves.ts:14983-14989, `onHit(target, source, move)`.
/// PRNG: none directly; `setStatus('slp')` runs the Sleep onStart, which draws `random(2,5)`
/// before this handler overwrites `time`/`startTime` with 3 (so the draw is still consumed).
/// Returns the falsy `setStatus` result unchanged, else undefined.
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let source = mon_arg(b, cx, 1);
    let mov = move_arg(b, cx, 2);
    let result = b.set_status(
        target,
        dex::CONDITION_SLP,
        Attribution::from_move(source, EffectRef::ActiveMove(mov)),
        false,
    );
    if !result.truthy() {
        return result;
    }
    // target.statusState.time = 3; target.statusState.startTime = 3;
    let cell = b.state.pokemon[target.0 as usize].status_state;
    if cell != CellId::NONE {
        let cell = &mut b.state.effects.cells[cell.0 as usize];
        cell.payload.words[0] = 3;
        cell.payload.words[1] = 3;
        cell.present |= SLP_START_TIME | SLP_TIME;
    }
    // this.heal(target.maxhp): "Aesthetic only as the healing happens after you fall asleep
    // in-game"; target/source/effect default from the running Hit event and move.
    let max_hp = b.state.pokemon[target.0 as usize].max_hp;
    b.heal(f64::from(max_hp), None, None, HealEffect::Context);
    Relay::Undefined
}
