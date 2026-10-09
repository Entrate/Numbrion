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
    pub handlers: [HandlerBuffer; EVENT_DEPTH],
    pub moves: [Option<ActiveMove>; MOVE_DEPTH],
    pub calls: [Option<CallArgs>; CALL_DEPTH],
    pub boosts: [OrderedBoosts; 8],
    pub types: [Option<Types>; 8],
    pub secondaries: [[Option<SecondaryScratch>; 4]; 8],
    pub event_depth: u8,
    pub move_depth: u8,
    pub call_depth: u8,
    pub current_frame: u8,
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
            move_depth: 0,
            call_depth: 0,
            current_frame: 255,
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
        todo!("stage 2B: stash_boosts")
    }
    /// Ports mutable boost relays (battle.ts:862-866). PRNG: none; drop before nested calls.
    pub fn scratch_boosts(&mut self, handle: u8) -> &mut OrderedBoosts {
        todo!("stage 2B: scratch_boosts")
    }
    /// Engine storage for pokemon.ts:2138-2146 Type relay arrays. PRNG: none.
    pub fn stash_types(&mut self, value: Types) -> u8 {
        todo!("stage 2B: stash_types")
    }
    /// Ports Type relays (pokemon.ts:2138-2146). PRNG: none; drop before nested calls.
    pub fn scratch_types(&mut self, handle: u8) -> &mut Types {
        todo!("stage 2B: scratch_types")
    }
    /// Engine storage for battle-actions.ts:1328 ModifySecondaries arrays. PRNG: none.
    pub fn stash_secondaries(&mut self, value: [Option<SecondaryScratch>; 4]) -> u8 {
        todo!("stage 2B: stash_secondaries")
    }
    /// Ports ModifySecondaries (battle-actions.ts:1328). PRNG: none.
    pub fn scratch_secondaries(&mut self, handle: u8) -> &mut [Option<SecondaryScratch>; 4] {
        todo!("stage 2B: scratch_secondaries")
    }
    /// Engine reclamation at battle.ts:939-943. PRNG: none; clear relay slot after use.
    pub fn release_relay(&mut self, value: Relay) {
        todo!("stage 2B: release_relay")
    }
}
