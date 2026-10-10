use crate::ids::{EffectId, Holder, MonId, SlotId};
pub const EFFECT_CAPACITY: usize = 640;
pub const VOLATILE_CAPACITY: usize = 45;
const RETIRED: u32 = 1 << 31;
pub mod present {
    pub const TARGET: u32 = 1 << 0;
    pub const SOURCE: u32 = 1 << 1;
    pub const SOURCE_SLOT: u32 = 1 << 2;
    pub const SOURCE_EFFECT: u32 = 1 << 3;
    pub const LINKED_STATUS: u32 = 1 << 4;
    pub const SLOT_CONDITION: u32 = 1 << 5;
    pub const PREFIXED_ID: u32 = 1 << 6;
    /// Truthy abilityState.ending marker; clear covers both absent and false.
    /// Queries only observe truthiness (pokemon.ts:872, field.ts:112).
    pub const ENDING: u32 = 1 << 7;
    pub const CUSTOM_START: u8 = 8;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct CellId(pub u16);
impl CellId {
    pub const NONE: Self = Self(u16::MAX);
}
/// Captured by collected listeners; a recycled arena index is a different JS object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRef {
    pub generation: u32,
    pub cell: CellId,
}
/// Four words, interpreted by per-effect accessors. `present` distinguishes absent
/// properties from false/zero. No Rust unions or references survive a snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct EffectPayload {
    pub words: [u32; 4],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct EffectCell {
    pub generation: u32,
    pub effect_order: u32,
    pub present: u32,
    pub payload: EffectPayload,
    pub id: EffectId,
    pub source_effect: EffectId,
    pub linked_status: EffectId,
    pub linked_mons: u16,
    pub owner: Holder,
    pub target: Holder,
    pub source: MonId,
    pub source_slot: SlotId,
    /// -1 absent; 0 is present zero (truthiness matters to residual expiry).
    pub duration: i16,
    // Free-list link while unallocated, listener pin count while allocated.
    free_next_or_pins: u16,
}
impl EffectCell {
    pub(crate) fn is_initial(&self, owner: Holder, target: Holder, id: EffectId) -> bool {
        *self == Self {
            generation: 1,
            owner,
            target,
            id,
            present: if target == Holder::NONE { 0 } else { present::TARGET },
            free_next_or_pins: 0,
            ..Self::EMPTY
        }
    }

    pub const EMPTY: Self = Self {
        generation: 0,
        effect_order: 0,
        present: 0,
        payload: EffectPayload { words: [0; 4] },
        id: EffectId::NONE,
        source_effect: EffectId::NONE,
        linked_status: EffectId::NONE,
        linked_mons: 0,
        owner: Holder::NONE,
        target: Holder::NONE,
        source: MonId::NONE,
        source_slot: SlotId::NONE,
        duration: -1,
        free_next_or_pins: u16::MAX,
    };
    /// Showdown clearEffectState preserves target and object identity, deletes other keys.
    pub fn clear(&mut self) {
        let (generation, owner, target, pins, retained_flags) = (
            self.generation,
            self.owner,
            self.target,
            self.free_next_or_pins,
            self.present & (RETIRED | present::TARGET),
        );
        *self = Self::EMPTY;
        self.generation = generation;
        self.owner = owner;
        self.target = target;
        self.free_next_or_pins = pins;
        self.present = retained_flags;
    }
}
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct EffectArena {
    pub cells: [EffectCell; EFFECT_CAPACITY],
    free_head: CellId,
    pub len: u16,
}
impl Default for EffectArena {
    fn default() -> Self {
        let mut cells = [EffectCell::EMPTY; EFFECT_CAPACITY];
        for (i, c) in cells.iter_mut().enumerate().take(EFFECT_CAPACITY - 1) {
            c.free_next_or_pins = i as u16 + 1;
        }
        Self {
            cells,
            free_head: CellId(0),
            len: 0,
        }
    }
}
impl EffectArena {
    pub fn alloc(
        &mut self,
        owner: Holder,
        target: Holder,
        id: EffectId,
        effect_order: u32,
    ) -> CellId {
        let index = self.free_head;
        assert_ne!(index, CellId::NONE, "Effect arena capacity exceeded");
        let c = &mut self.cells[index.0 as usize];
        self.free_head = CellId(c.free_next_or_pins);
        let generation = c
            .generation
            .checked_add(1)
            .expect("Effect generation overflow");
        *c = EffectCell {
            generation,
            owner,
            target,
            id,
            effect_order,
            present: if target != Holder::NONE {
                present::TARGET
            } else {
                0
            },
            free_next_or_pins: 0,
            ..EffectCell::EMPTY
        };
        self.len += 1;
        index
    }
    pub fn release(&mut self, cell: CellId) {
        let c = &mut self.cells[cell.0 as usize];
        assert_ne!(c.owner, Holder::NONE, "double effect release");
        assert_eq!(c.present & RETIRED, 0, "double effect retirement");
        if c.free_next_or_pins != 0 {
            c.present |= RETIRED;
            return;
        }
        self.free(cell);
    }
    fn free(&mut self, cell: CellId) {
        let c = &mut self.cells[cell.0 as usize];
        *c = EffectCell {
            generation: c.generation,
            free_next_or_pins: self.free_head.0,
            ..EffectCell::EMPTY
        };
        self.free_head = cell;
        self.len -= 1;
    }
    /// Collection pins the original object even if a later callback removes it.
    pub fn pin(&mut self, cell: CellId) -> CellRef {
        let c = &mut self.cells[cell.0 as usize];
        assert_ne!(c.owner, Holder::NONE);
        c.free_next_or_pins = c
            .free_next_or_pins
            .checked_add(1)
            .expect("listener pin overflow");
        self.capture(cell)
    }
    pub fn unpin(&mut self, reference: CellRef) {
        let c = &mut self.cells[reference.cell.0 as usize];
        assert_eq!(c.generation, reference.generation);
        assert_ne!(c.free_next_or_pins, 0);
        c.free_next_or_pins -= 1;
        if c.free_next_or_pins == 0 && c.present & RETIRED != 0 {
            self.free(reference.cell);
        }
    }
    pub fn capture(&self, cell: CellId) -> CellRef {
        CellRef {
            cell,
            generation: self.cells[cell.0 as usize].generation,
        }
    }
    pub fn is_live(&self, reference: CellRef) -> bool {
        let c = &self.cells[reference.cell.0 as usize];
        c.owner != Holder::NONE && c.present & RETIRED == 0 && c.generation == reference.generation
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct EffectList<const N: usize> {
    pub cells: [CellId; N],
    pub len: u16,
}
impl<const N: usize> Default for EffectList<N> {
    fn default() -> Self {
        Self {
            cells: [CellId::NONE; N],
            len: 0,
        }
    }
}
impl<const N: usize> EffectList<N> {
    pub fn as_slice(&self) -> &[CellId] {
        &self.cells[..self.len as usize]
    }
    pub fn push(&mut self, cell: CellId) {
        assert!((self.len as usize) < N, "Effect list capacity exceeded");
        self.cells[self.len as usize] = cell;
        self.len += 1;
    }
    pub fn remove(&mut self, index: usize) -> CellId {
        let len = self.len as usize;
        assert!(index < len);
        let cell = self.cells[index];
        self.cells.copy_within(index + 1..len, index);
        self.len -= 1;
        self.cells[self.len as usize] = CellId::NONE;
        cell
    }
}
const _: () = assert!(core::mem::size_of::<EffectCell>() == 44);
const _: () = assert!(core::mem::size_of::<EffectPayload>() == 16);
