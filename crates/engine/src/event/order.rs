//! Exact selection sort, not Rust sort; sim/battle.ts:408-467.
#![allow(unused_variables)]
use super::{Listener, Priority};
use crate::{Battle, log::LogSink, prng::Prng, state::choices::Action};
#[derive(Clone, Copy, Debug)]
pub enum SortOrder {
    Priority,
    Redirect,
    LeftToRight,
}
pub trait SpeedSortable: Copy {
    /// Ports battle.ts:408-415. PRNG: none; all speed/priority values are cached.
    fn sort_key(&self) -> Priority;
}
impl SpeedSortable for Listener {
    fn sort_key(&self) -> Priority {
        self.priority
    }
}
impl SpeedSortable for Action {
    fn sort_key(&self) -> Priority {
        Priority {
            order: self.order as f64,
            priority: self.priority, // L already adds fractionalPriority at battle.ts:2651.
            speed: self.speed as f64,
            sub_order: 0.0,
            effect_order: 0,
            index: 0,
            redirect_order: None,
        }
    }
}
/// Ports sim/battle.ts:433-467. PRNG: shuffle each selected tied block (n-1 draws),
/// including ties with identical object keys. Stable sorts use comparator separately.
pub fn speed_sort<T: SpeedSortable>(prng: &mut Prng, list: &mut [T], order: SortOrder) {
    todo!("stage 2B: speed_sort")
}
/// Ports sim/battle.ts:408-430. PRNG: none. Return JS comparator sign, preserving
/// falsy zero order, explicit default keys, quarter-speed and redirect holder order.
pub fn compare_priority(a: Priority, b: Priority, order: SortOrder) -> f64 {
    todo!("stage 2B: compare_priority")
}
impl<L: LogSink> Battle<L> {
    /// Ports sim/battle.ts:954-1020. PRNG: none; cache speed before handlers mutate it.
    pub fn resolve_priority(&self, listener: Listener) -> Listener {
        todo!("stage 2B: resolve_priority")
    }
}
