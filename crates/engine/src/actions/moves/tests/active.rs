//! Active object identity and frame lifetime across recursive inside callers.
use super::fixture::*;
use crate::{actions::MoveInput, state::scratch::move_runtime as rt};

#[test]
fn active_input_reuses_identity_and_explicit_copy_preserves_deleted_fields() {
    let mut b = battle_with([0; 4]);
    let h = b.get_active_move(MoveInput::Dex(move_id("closecombat")));
    assert_eq!(b.get_active_move(MoveInput::Active(h)), h);
    {
        let m = b.active_move_mut(h);
        m.base_power = 0.0;
        m.flags = 0;
        m.recoil = None;
        m.drain = None;
        m.self_effect = None;
        m.secondaries = [None; 4];
        m.secondary_count = 0;
        m.effects.boosts = Default::default();
        m.runtime_flags |= rt::HAS_BOUNCED;
    }
    let copy = b.copy_active_move(h);
    let m = b.active_move(copy);
    assert_eq!(m.base_power, 0.0);
    assert_eq!(m.flags, 0);
    assert!(m.self_effect.is_none() && m.recoil.is_none() && m.drain.is_none());
    assert_eq!(m.effects.boosts.len, 0);
    assert!(m.runtime_flags & rt::HAS_BOUNCED != 0);
    assert_ne!(copy, h);
    b.release_active_move(copy);
    b.release_active_move(h);
    assert!(b.scratch.moves.iter().all(Option::is_none));
}

#[test]
fn completed_nested_frames_are_reclaimed_at_clear_without_releasing_outer_owner() {
    let mut b = battle_with([0; 4]);
    let outer = b.get_active_move(MoveInput::Dex(move_id("psychic")));
    let inner = b.get_active_move(MoveInput::Dex(move_id("thunderbolt")));
    b.set_active_move(Some(inner), Some(ACTIVES[0]), Some(ACTIVES[2]));
    b.release_active_move(inner);
    assert!(b.scratch.moves[inner.0 as usize].is_some());
    b.clear_active_move(false);
    assert_eq!(b.state.last_move, move_id("thunderbolt"));
    assert!(b.scratch.moves[inner.0 as usize].is_none());
    assert!(b.scratch.moves[outer.0 as usize].is_some());
    b.release_active_move(outer);
    assert!(b.scratch.moves.iter().all(Option::is_none));
}
