//! Small shared authoring helpers. No allocation, discovery, or PRNG draws.
use crate::{
    Battle,
    dex::{self, EffectData},
    event::*,
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{Status, scratch::ActiveMove},
};
/// Compile-time lookup for optional effects outside the fixed scope; NONE never matches an ability.
pub const fn optional_id(table: &[EffectData], key: &str) -> EffectId {
    let mut i = 0;
    while i < table.len() {
        let a = table[i].key.as_bytes();
        let b = key.as_bytes();
        let mut j = 0;
        let mut same = a.len() == b.len();
        while same && j < a.len() {
            same = a[j] == b[j];
            j += 1;
        }
        if same {
            return table[i].id;
        }
        i += 1;
    }
    EffectId::NONE
}
pub fn mon_arg<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> MonId {
    Battle::<L>::arg_mon(b.event_arg(cx, index)).expect("callback requires Pokemon argument")
}
pub fn relay_number<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> f64 {
    match b.event_arg(cx, index) {
        EventArg::Relay(Relay::Number(n)) | EventArg::Number(n) => n,
        _ => panic!("callback requires numeric argument"),
    }
}
pub fn move_arg<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> u8 {
    match b.event_arg(cx, index) {
        EventArg::Move(i)
        | EventArg::Effect(EffectRef::ActiveMove(i))
        | EventArg::Relay(Relay::ActiveMove(i)) => i,
        _ => panic!("callback requires live move"),
    }
}
pub fn move_overlay<L: LogSink>(b: &Battle<L>, i: u8) -> &ActiveMove {
    b.scratch.moves[i as usize]
        .as_ref()
        .expect("released active move")
}
pub fn has_ability<L: LogSink>(b: &Battle<L>, m: MonId, id: EffectId) -> bool {
    id != EffectId::NONE && b.state.pokemon[m.0 as usize].ability == id && !b.ignoring_ability(m)
}
/// data/conditions.ts:5-16,24-30,57-65,95-101,139-145,161-171. PRNG: none.
pub fn log_status<L: LogSink>(
    b: &mut Battle<L>,
    cx: HookCtx,
    status: Status,
    command: &'static str,
) {
    let m = mon_arg(b, cx, 0);
    let effect = match b.event_arg(cx, 2) {
        EventArg::Effect(e) => e,
        _ => EffectRef::None,
    };
    let id = b.event_effect_id(effect);
    let args = [LogArg::Mon(m), LogArg::Status(status)];
    if (status == Status::Burn && id == dex::ITEM_FLAMEORB)
        || (status == Status::Toxic && id == dex::ITEM_TOXICORB)
        || (status == Status::Sleep
            && b.event_effect_type(effect) == dex::EffectType::Move
            && effect != EffectRef::None)
    {
        b.add(LogEntry::new(command, &args, &[LogTag::From(effect)]));
    } else if effect != EffectRef::None && b.event_effect_type(effect) == dex::EffectType::Ability {
        let source = mon_arg(b, cx, 1);
        b.add(LogEntry::new(
            command,
            &args,
            &[LogTag::From(effect), LogTag::Of(source)],
        ));
    } else {
        b.add(LogEntry::new(command, &args, &[]));
    }
}
