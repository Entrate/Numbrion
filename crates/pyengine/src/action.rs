//! The fixed per-slot discrete action space (docs/design/TRAINING-API.md).
//!
//! One decision of one side is a pair of codes `(a0, a1)`, one for each active slot, each in
//! `0..N_ACTIONS`:
//!
//! ```text
//!   0..40   move   : code = slot * 10 + target * 2 + tera     slot 0..4, target 0..5, tera 0..2
//!   40..46  switch : code = 40 + party index                  party index 0..6 (see below)
//!   46      pass
//! ```
//!
//! `target` is `signed Showdown target location + 2`, i.e. 0 = -2 (own slot b), 1 = -1 (own slot a),
//! 2 = no target (auto), 3 = +1 (foe slot a), 4 = +2 (foe slot b). A move that takes no target
//! (spread, self, field, locked, Struggle) is only legal with target code 2.
//!
//! `slot` is the index of the move in the Pokemon's request `moves` list (stable for the Pokemon's
//! whole life, unlike a compacted list of the currently enabled moves).
//!
//! Switches use the *current* party index of the request's `side.pokemon` array (what `switch N` means
//! in Showdown, 0-based here). Indices 0 and 1 are the active Pokemon and can only be named by a
//! Revival Blessing revive.
//!
//! A forced move (Outrage-style lock, charging move, Recharge, Struggle) has no choice to make; its single
//! canonical code is `move slot 0, target 2, tera 0`. Aliases that the engine would also accept
//! (a Tera suffix on a locked move, ...) are masked out so that every behaviour has exactly one code.

pub const N_MOVE_SLOTS: usize = 4;
pub const N_TARGETS: usize = 5;
/// Target code meaning "no target / automatic".
pub const TARGET_AUTO: u8 = 2;
pub const N_MOVE_ACTIONS: usize = N_MOVE_SLOTS * N_TARGETS * 2;
pub const SWITCH_BASE: usize = N_MOVE_ACTIONS;
pub const N_SWITCH: usize = 6;
pub const PASS: usize = SWITCH_BASE + N_SWITCH;
pub const N_ACTIONS: usize = PASS + 1;

const _: () = assert!(N_ACTIONS == 47 && N_ACTIONS <= 64);

/// A decoded action code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Move { slot: u8, target: u8, tera: bool },
    Switch(u8),
    Pass,
}

#[inline]
pub const fn move_action(slot: u8, target: u8, tera: bool) -> usize {
    slot as usize * (N_TARGETS * 2) + target as usize * 2 + tera as usize
}

#[inline]
pub const fn switch_action(party: u8) -> usize {
    SWITCH_BASE + party as usize
}

/// The canonical code of a forced move.
pub const FORCED_MOVE: usize = move_action(0, TARGET_AUTO, false);

#[inline]
pub fn decode(a: usize) -> Option<Act> {
    if a < N_MOVE_ACTIONS {
        Some(Act::Move {
            slot: (a / (N_TARGETS * 2)) as u8,
            target: (a / 2 % N_TARGETS) as u8,
            tera: a % 2 == 1,
        })
    } else if a < PASS {
        Some(Act::Switch((a - SWITCH_BASE) as u8))
    } else if a == PASS {
        Some(Act::Pass)
    } else {
        None
    }
}

#[inline]
pub fn encode(act: Act) -> usize {
    match act {
        Act::Move { slot, target, tera } => move_action(slot, target, tera),
        Act::Switch(p) => switch_action(p),
        Act::Pass => PASS,
    }
}

/// Signed Showdown target location of a target code.
#[inline]
pub const fn target_loc(code: u8) -> i8 {
    code as i8 - 2
}

/// Joint-constraint classes. Whether two slot actions can be chosen together depends only on the class
/// of each (non-Tera move, Tera move, switch to party index p, pass), which is what lets masks be computed
/// with a handful of checks instead of 47 x 47.
pub const N_CLASSES: usize = 9;
const CLASS_TERA: usize = 1;
const CLASS_SWITCH0: usize = 2;
const CLASS_PASS: usize = 2 + N_SWITCH;

#[inline]
pub const fn class_of(a: usize) -> usize {
    if a < N_MOVE_ACTIONS {
        if a % 2 == 1 { CLASS_TERA } else { 0 }
    } else if a < PASS {
        CLASS_SWITCH0 + (a - SWITCH_BASE)
    } else {
        CLASS_PASS
    }
}

const fn class_masks() -> [u64; N_CLASSES] {
    let mut m = [0u64; N_CLASSES];
    let mut a = 0;
    while a < N_ACTIONS {
        m[class_of(a)] |= 1 << a;
        a += 1;
    }
    m
}

/// Bit set of the action codes of each class.
pub const CLASS_MASKS: [u64; N_CLASSES] = class_masks();

/// Every code `0..N_ACTIONS`.
pub const ALL_ACTIONS: u64 = (1u64 << N_ACTIONS) - 1;

/// The class of a code, with the switch party index for switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Move { tera: bool },
    Switch(u8),
    Pass,
}

#[inline]
pub fn classify(a: usize) -> Class {
    match decode(a) {
        Some(Act::Move { tera, .. }) => Class::Move { tera },
        Some(Act::Switch(p)) => Class::Switch(p),
        Some(Act::Pass) | None => Class::Pass,
    }
}

/// Human readable name of a code, e.g. `move 2 target +1 tera`, `switch 4`, `pass`.
pub fn describe(a: usize) -> String {
    match decode(a) {
        Some(Act::Move { slot, target, tera }) => {
            let loc = target_loc(target);
            let mut s = format!("move {}", slot + 1);
            if loc != 0 {
                s.push_str(&format!(" target {loc:+}"));
            }
            if tera {
                s.push_str(" tera");
            }
            s
        }
        Some(Act::Switch(p)) => format!("switch {}", p + 1),
        Some(Act::Pass) => "pass".to_string(),
        None => format!("invalid({a})"),
    }
}

/// Iterate the set bits of a mask as action codes.
#[inline]
pub fn bits(mut m: u64) -> impl Iterator<Item = usize> {
    std::iter::from_fn(move || {
        if m == 0 {
            None
        } else {
            let i = m.trailing_zeros() as usize;
            m &= m - 1;
            Some(i)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for a in 0..N_ACTIONS {
            assert_eq!(encode(decode(a).unwrap()), a);
        }
        assert!(decode(N_ACTIONS).is_none());
        assert_eq!(FORCED_MOVE, 4);
        assert_eq!(move_action(3, 4, true), 39);
    }

    #[test]
    fn classes_partition_the_space() {
        let mut seen = 0u64;
        for m in CLASS_MASKS {
            assert_eq!(seen & m, 0);
            seen |= m;
        }
        assert_eq!(seen, ALL_ACTIONS);
        assert_eq!(class_of(move_action(1, 0, true)), CLASS_TERA);
        assert_eq!(class_of(move_action(1, 0, false)), 0);
        assert_eq!(class_of(switch_action(3)), CLASS_SWITCH0 + 3);
        assert_eq!(class_of(PASS), CLASS_PASS);
    }
}
