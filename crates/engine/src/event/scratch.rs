//! Worker scratch is not part of a Copy snapshot (battle.ts:374,575,762).
//! Only construction allocates. Scratch reset at a flushed decision boundary draws nothing.
use super::*;
use crate::{
    actions::{MoveHandle, Types},
    ids::*,
    state::scratch::{ActiveMove, OrderedBoosts, SecondaryScratch},
};
#[derive(Debug)]
pub struct Scratch {
    pub frames: [Option<EventFrame>; EVENT_DEPTH],
    pub handlers: [HandlerBuffer; CALL_DEPTH],
    pub moves: [Option<ActiveMove>; MOVE_DEPTH],
    pub calls: [Option<CallArgs>; CALL_DEPTH],
    pub boosts: [OrderedBoosts; 8],
    pub types: [Option<Types>; 8],
    pub secondaries: [[Option<SecondaryScratch>; 4]; 8],
    pub event_depth: u8,
    pub handler_depth: u8,
    pub current_context: Option<HookCtx>,
    pub boosts_used: u8,
    pub secondaries_used: u8,
    pub move_depth: u8,
    pub call_depth: u8,
    pub current_frame: u8,
    pub initial_modifier: u32,
    pub initial_modifier_present: bool,
    pub current_effect: EffectRef,
    pub current_state: Option<crate::state::CellRef>,
    pub active_move: MoveHandle,
    pub active_pokemon: MonId,
    pub active_target: MonId,
    /// Counts protocol entries, including three for split; independent of sink ENABLED.
    pub unsent_lines: u32,
}
impl Default for Scratch {
    fn default() -> Self {
        Self {
            frames: [None; EVENT_DEPTH],
            handlers: core::array::from_fn(|_| HandlerBuffer::default()),
            moves: [None; MOVE_DEPTH],
            calls: [None; CALL_DEPTH],
            boosts: [OrderedBoosts::default(); 8],
            types: [None; 8],
            secondaries: [[None; 4]; 8],
            event_depth: 0,
            handler_depth: 0,
            current_context: None,
            boosts_used: 0,
            secondaries_used: 0,
            move_depth: 0,
            call_depth: 0,
            current_frame: 255,
            initial_modifier: 0,
            initial_modifier_present: false,
            current_effect: EffectRef::None,
            current_state: None,
            active_move: MoveHandle::NONE,
            active_pokemon: MonId::NONE,
            active_target: MonId::NONE,
            unsent_lines: 4,
        }
    }
}
#[allow(unused_variables)]
impl<L: crate::log::LogSink> crate::Battle<L> {
    /// Engine storage for battle.ts:862-866 mutable relay objects. PRNG: none.
    pub fn stash_boosts(&mut self, value: OrderedBoosts) -> u8 {
        let i = (!self.scratch.boosts_used).trailing_zeros() as usize;
        assert!(i < 8, "boost relay capacity exceeded");
        self.scratch.boosts_used |= 1 << i;
        self.scratch.boosts[i] = value;
        i as u8
    }
    /// Ports mutable boost relays (battle.ts:862-866). PRNG: none; drop before nested calls.
    pub fn scratch_boosts(&mut self, handle: u8) -> &mut OrderedBoosts {
        assert!(handle < 8 && self.scratch.boosts_used & (1 << handle) != 0);
        &mut self.scratch.boosts[handle as usize]
    }
    /// Engine storage for pokemon.ts:2138-2146 Type relay arrays. PRNG: none.
    pub fn stash_types(&mut self, value: Types) -> u8 {
        let i = self
            .scratch
            .types
            .iter()
            .position(Option::is_none)
            .expect("type relay capacity exceeded");
        self.scratch.types[i] = Some(value);
        i as u8
    }
    /// Ports Type relays (pokemon.ts:2138-2146). PRNG: none; drop before nested calls.
    pub fn scratch_types(&mut self, handle: u8) -> &mut Types {
        self.scratch.types[handle as usize]
            .as_mut()
            .expect("released type relay")
    }
    /// Engine storage for battle-actions.ts:1328 ModifySecondaries arrays. PRNG: none.
    pub fn stash_secondaries(&mut self, value: [Option<SecondaryScratch>; 4]) -> u8 {
        let i = (!self.scratch.secondaries_used).trailing_zeros() as usize;
        assert!(i < 8, "secondary relay capacity exceeded");
        self.scratch.secondaries_used |= 1 << i;
        self.scratch.secondaries[i] = value;
        i as u8
    }
    /// Ports ModifySecondaries (battle-actions.ts:1328). PRNG: none.
    pub fn scratch_secondaries(&mut self, handle: u8) -> &mut [Option<SecondaryScratch>; 4] {
        assert!(handle < 8 && self.scratch.secondaries_used & (1 << handle) != 0);
        &mut self.scratch.secondaries[handle as usize]
    }
    /// Engine reclamation at battle.ts:939-943. PRNG: none; clear relay slot after use.
    pub fn release_relay(&mut self, value: Relay) {
        match value {
            Relay::Boosts(i) => self.scratch.boosts_used &= !(1 << i),
            Relay::Types(i) => self.scratch.types[i as usize] = None,
            Relay::Secondaries(i) => self.scratch.secondaries_used &= !(1 << i),
            _ => {}
        }
    }
}
