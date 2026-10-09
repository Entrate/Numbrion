//! Frame-local 4096ths arithmetic; absent modifiers retain JS undefined semantics.
use crate::{Battle, log::LogSink, math};
impl<L: LogSink> Battle<L> {
    /// Chain into current frame using verified 4096ths rounding.
    /// Ports `sim/battle.ts:2325-2334`. PRNG: none.
    pub fn chain_modify(&mut self, numerator: f64, denominator: f64) -> () {
        let next = math::trunc_f64(numerator * 4096.0 / denominator);
        let previous = self.current_modifier();
        self.set_current_modifier(math::chain(previous, next));
    }
    /// Port JavaScript truncation/rounding; never Rust integer rounding by accident.
    /// Ports `sim/battle.ts:2336-2348`. PRNG: none.
    pub fn modify(&self, value: f64, numerator: f64, denominator: f64) -> f64 {
        let modifier = math::trunc_f64(numerator * 4096.0 / denominator);
        apply_modifier(value, modifier)
    }
    /// Apply current frame modifier to a numeric relay.
    /// Ports `sim/battle.ts:2385-2389`. PRNG: none.
    pub fn final_modify(&mut self, relay: f64) -> f64 {
        let modified = apply_modifier(relay, self.current_modifier());
        self.set_current_modifier(math::MOD_ONE);
        modified
    }

    fn current_modifier(&self) -> math::Mod4096 {
        if self.scratch.current_frame == 255 {
            if self.scratch.initial_modifier_present {
                self.scratch.initial_modifier
            } else {
                0 // trunc(undefined * 4096) == 0
            }
        } else {
            let frame = self.scratch.frames[self.scratch.current_frame as usize]
                .as_ref()
                .expect("current event frame missing");
            if frame.modifier_present {
                frame.modifier
            } else {
                0
            }
        }
    }

    fn set_current_modifier(&mut self, modifier: math::Mod4096) {
        if self.scratch.current_frame == 255 {
            self.scratch.initial_modifier = modifier;
            self.scratch.initial_modifier_present = true;
        } else {
            let frame = self.scratch.frames[self.scratch.current_frame as usize]
                .as_mut()
                .expect("current event frame missing");
            frame.modifier = modifier;
            frame.modifier_present = true;
        }
    }
}

#[inline]
fn apply_modifier(value: f64, modifier: math::Mod4096) -> f64 {
    // Preserve fractions before the inner ToUint32; math::modify accepts i64.
    let scaled = math::trunc_f64(value * modifier as f64);
    math::trunc_f64((scaled as f64 + 2047.0) / 4096.0) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        event::{EffectRef, EventArg, EventFrame, Relay},
        ids::EventId,
        log::NoLog,
    };

    fn battle() -> Battle<NoLog> {
        let packed = "Pikachu||lightball|static|thunderbolt|Serious||M|||100|,,,,,Electric";
        Battle::new([1, 2, 3, 4], packed, packed).unwrap()
    }
    fn frame(modifier: u32, present: bool) -> EventFrame {
        EventFrame {
            event: EventId::BasePower,
            target: EventArg::Undefined,
            source: EventArg::Undefined,
            source_effect: EffectRef::None,
            relay: Relay::Number(100.),
            modifier,
            modifier_present: present,
            has_relay: true,
            fast_exit: false,
            args: [EventArg::Undefined; 4],
            arg_count: 0,
        }
    }

    #[test]
    fn modify_preserves_fractional_inputs_and_js_uint32_wrapping() {
        let battle = battle();
        // Pinned Battle.prototype.modify oracle, including fractional and wrap cases.
        for (value, numerator, denominator, expected) in [
            (57.5, 1.5, 1., 86.),
            (-0.25, 2., 1., 1048575.),
            (4294967296.75, 3., 2., 1.),
            (123.456, 5325., 4096., 160.),
            (f64::NAN, 1., 1., 0.),
            (f64::INFINITY, 1., 1., 0.),
            (100., 1.3, 1., 130.),
            (100., 1., 0., 0.),
            (100., -1., 1., 1048476.),
        ] {
            assert_eq!(battle.modify(value, numerator, denominator), expected);
        }
        assert_eq!(battle.seed(), [1, 2, 3, 4]);
    }

    #[test]
    fn chain_modify_and_final_modify_mutate_only_current_frame() {
        let mut battle = battle();
        battle.scratch.frames[0] = Some(frame(6144, true));
        battle.scratch.frames[1] = Some(frame(4096, true));
        battle.scratch.current_frame = 1;
        for (numerator, denominator, expected) in [
            (1.5, 1., 6144),
            (5325., 4096., 7988),
            (2., 3., 5324),
            (0.75, 1., 3993),
        ] {
            battle.chain_modify(numerator, denominator);
            assert_eq!(battle.scratch.frames[1].unwrap().modifier, expected);
        }
        assert_eq!(battle.final_modify(111.), 108.);
        assert_eq!(battle.scratch.frames[1].unwrap().modifier, 4096);
        assert_eq!(battle.scratch.frames[0].unwrap().modifier, 6144);
        battle.scratch.current_frame = 0;
        assert_eq!(battle.final_modify(100.), 150.);
        assert_eq!(battle.scratch.frames[0].unwrap().modifier, 4096);
        assert_eq!(battle.seed(), [1, 2, 3, 4]);
    }

    #[test]
    fn absent_single_event_and_initial_modifiers_follow_undefined_semantics() {
        let mut battle = battle();
        assert!(!battle.scratch.initial_modifier_present);
        battle.chain_modify(1.5, 1.);
        assert!(battle.scratch.initial_modifier_present);
        assert_eq!(battle.scratch.initial_modifier, 0);
        assert_eq!(battle.final_modify(100.), 0.);
        assert_eq!(battle.scratch.initial_modifier, 4096);
        battle.scratch.frames[0] = Some(frame(4096, false));
        battle.scratch.current_frame = 0;
        battle.chain_modify(1.5, 1.);
        assert_eq!(battle.scratch.frames[0].unwrap().modifier, 0);
        assert!(battle.scratch.frames[0].unwrap().modifier_present);
        assert_eq!(battle.final_modify(100.), 0.);
        assert_eq!(battle.scratch.frames[0].unwrap().modifier, 4096);
        assert_eq!(battle.scratch.initial_modifier, 4096);
        assert_eq!(battle.seed(), [1, 2, 3, 4]);
    }
}
