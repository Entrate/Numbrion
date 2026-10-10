mod support;
use super::ID;
use crate::{
    Battle, dex,
    event::*,
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogView, MoveLineEdit},
    state::mon_flags,
};
#[derive(Default)]
struct Capture {
    mons: Vec<(String, MonId)>,
}
impl LogSink for Capture {
    const ENABLED: bool = true;
    fn emit(&mut self, _: LogView<'_>, e: LogEntry<'_>) {
        assert!(e.tags.is_empty());
        let LogArg::Mon(m) = e.args[0] else {
            panic!("expected mon")
        };
        if e.command == "-ability" {
            assert!(
                matches!(e.args,[LogArg::Mon(_),LogArg::Effect(EffectRef::Dex(id)),LogArg::Text("boost")] if *id==ID)
            );
        } else {
            assert_eq!(e.command, "-immune");
            assert_eq!(e.args.len(), 1);
        }
        self.mons.push((e.command.into(), m));
    }
    fn edit_move(&mut self, _: LogView<'_>, _: MoveLineEdit<'_>) {
        panic!("unexpected edit")
    }
}
#[test]
fn real_target_enumeration_preserves_substitute_and_zero_stat_guards() {
    let packed = "Pikachu||lightball|static|thunderbolt|Serious||M|||100|,,,,,Electric";
    for first_hp in [40, 0] {
        let mut b = Battle::with_log([1, 2, 3, 4], packed, packed, Capture::default()).unwrap();
        b.state.sides[0].active = [MonId(0), MonId::NONE];
        b.state.sides[1].active = [MonId(6), MonId(7)];
        for m in [0, 6, 7] {
            let p = &mut b.state.pokemon[m];
            p.flags = mon_flags::ACTIVE;
            p.hp = 40;
            p.ability = dex::ABILITY_STATIC;
            p.stored_stats = [0; 5];
        }
        b.state.pokemon[6].hp = first_hp;
        for m in [6, 7] {
            let c = b.state.effects.alloc(
                Holder::mon(MonId(m)),
                Holder::mon(MonId(m)),
                dex::CONDITION_SUBSTITUTE,
                0,
            );
            b.state.pokemon[m as usize].volatiles.push(c);
        }
        let cell = b
            .state
            .effects
            .alloc(Holder::mon(MonId(0)), Holder::mon(MonId(0)), ID, 0);
        b.scratch.current_state = Some(b.state.effects.pin(cell));
        let manifest = &dex::MANIFESTS[ID.0 as usize];
        let i = manifest
            .hooks()
            .iter()
            .position(|h| h.key == "onStart")
            .unwrap();
        let result = b.call_hook(
            dex::HookId(manifest.hooks_start + i as u16),
            CallArgs {
                values: [
                    EventArg::Holder(Holder::mon(MonId(0))),
                    EventArg::Undefined,
                    EventArg::Undefined,
                    EventArg::Undefined,
                ],
                len: 1,
            },
        );
        assert_eq!(result, Relay::Undefined);
        if ID == dex::ABILITY_INTIMIDATE {
            let mut expected = vec![("-ability".into(), MonId(0))];
            if first_hp != 0 {
                expected.push(("-immune".into(), MonId(6)));
            }
            expected.push(("-immune".into(), MonId(7)));
            assert_eq!(b.log.mons, expected);
        } else {
            assert!(b.log.mons.is_empty());
        }
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}
