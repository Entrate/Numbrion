mod support;
use super::ID;
use crate::{
    Battle, dex,
    event::*,
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag, LogView, MoveLineEdit},
};
#[derive(Default)]
struct Capture {
    lines: Vec<String>,
}
impl LogSink for Capture {
    const ENABLED: bool = true;
    fn emit(&mut self, _: LogView<'_>, e: LogEntry<'_>) {
        assert_eq!(e.command, "-immune");
        assert!(matches!(e.args, [LogArg::Mon(MonId(0))]));
        assert!(matches!(e.tags,[LogTag::From(EffectRef::Dex(id))] if *id==ID));
        self.lines.push(e.command.into());
    }
    fn edit_move(&mut self, _: LogView<'_>, _: MoveLineEdit<'_>) {
        panic!("unexpected edit")
    }
}
#[test]
fn thermal_status_guard_emits_structural_from_tag_only_for_primary_status() {
    let packed = "Pikachu||lightball|static|thunderbolt|Serious||M|||100|,,,,,Electric";
    for (primary, synthetic) in [(false, false), (true, false), (false, true)] {
        let mut b = Battle::with_log([1, 2, 3, 4], packed, packed, Capture::default()).unwrap();
        b.scratch.moves[0] = Some(support::active());
        b.scratch.moves[0].as_mut().unwrap().effects.status = if primary {
            dex::CONDITION_BRN
        } else {
            EffectId::NONE
        };
        let m = &dex::MANIFESTS[ID.0 as usize];
        let i = m
            .hooks()
            .iter()
            .position(|h| h.key == "onSetStatus")
            .unwrap();
        let hook = dex::HookId(m.hooks_start + i as u16);
        let args = CallArgs {
            values: [
                EventArg::Effect(EffectRef::Dex(dex::CONDITION_BRN)),
                EventArg::Holder(Holder::mon(MonId(0))),
                EventArg::Holder(Holder::mon(MonId(6))),
                if synthetic {
                    EventArg::Effect(EffectRef::Synchronize(dex::CONDITION_BRN))
                } else {
                    EventArg::Move(0)
                },
            ],
            len: 4,
        };
        let before = b.scratch.unsent_lines;
        assert_eq!(b.call_hook(hook, args), Relay::Bool(false));
        assert_eq!(b.log.lines.len(), usize::from(primary || synthetic));
        assert_eq!(
            b.scratch.unsent_lines - before,
            u32::from(primary || synthetic)
        );
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}
