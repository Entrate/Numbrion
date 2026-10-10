//! The entire mutable simulation snapshot. No heap, strings, or borrowed pointers.
pub mod choices;
pub mod effects;
pub mod scratch;
use crate::{dex::MoveTarget, ids::*, prng::Prng};
pub use effects::*;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum ResultFlag {
    #[default]
    Undefined,
    Null,
    False,
    True,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
    #[default]
    None,
    Burn,
    Paralysis,
    Sleep,
    Freeze,
    Poison,
    Toxic,
    Fainted,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Trapped {
    #[default]
    No,
    Yes,
    Hidden,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct MoveSlot {
    pub id: EffectId,
    pub disabled_source: EffectId,
    pub pp: u8,
    pub max_pp: u8,
    pub target: MoveTarget,
    pub flags: u8,
}
/// Present + numeric-present bits, positive-damage-this-turn bit. Latest qualifying
/// records reproduce every scoped consumer without an unbounded attack history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct AttackRecord {
    pub last_seq: u32,
    pub numeric_seq: u32,
    pub last_damage: u32,
    pub numeric_damage: u32,
    pub last_move: EffectId,
    pub last_slot: SlotId,
    pub numeric_slot: SlotId,
    pub flags: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Pokemon {
    pub flags: u32,
    pub stellar_boosted_types: u32,
    pub stored_stats: [u16; 5],
    pub base_stored_stats: [u16; 5],
    pub base_move_slots: [MoveSlot; 4],
    pub virtual_move_slots: [MoveSlot; 4],
    pub attacks: [AttackRecord; 12],
    pub volatiles: EffectList<VOLATILE_CAPACITY>,
    pub species: EffectId,
    pub base_species: EffectId,
    pub ability: EffectId,
    pub base_ability: EffectId,
    pub item: EffectId,
    pub last_item: EffectId,
    pub status_state: CellId,
    pub ability_state: CellId,
    pub item_state: CellId,
    pub species_state: CellId,
    pub last_move: EffectId,
    pub switch_flag: EffectId,
    pub hp: u16,
    pub max_hp: u16,
    pub weighthg: u16,
    pub speed: u16,
    pub active_turns: u32,
    pub active_move_actions: u32,
    pub times_attacked: u32,
    pub types: [TypeId; 2],
    pub apparent_types: [TypeId; 2],
    pub boosts: [i8; 7],
    pub added_type: TypeId,
    pub terastallized: TypeId,
    pub position: u8,
    pub illusion: MonId,
    pub move_count: u8,
    pub virtual_move_count: u8,
    pub last_move_target_loc: i8,
    pub status: Status,
    pub trapped: Trapped,
    pub move_this_turn_result: ResultFlag,
    pub move_last_turn_result: ResultFlag,
    pub show_cure: ResultFlag,
}
pub mod mon_flags {
    pub const ACTIVE: u32 = 1 << 0;
    pub const FAINTED: u32 = 1 << 1;
    pub const FAINT_QUEUED: u32 = 1 << 2;
    pub const TRANSFORMED: u32 = 1 << 3;
    pub const KNOWN_TYPE: u32 = 1 << 4;
    pub const NEWLY_SWITCHED: u32 = 1 << 5;
    pub const BEING_CALLED_BACK: u32 = 1 << 6;
    pub const FORCE_SWITCH: u32 = 1 << 7;
    pub const SKIP_BEFORE_SWITCH_OUT: u32 = 1 << 8;
    pub const STATS_RAISED: u32 = 1 << 9;
    pub const STATS_LOWERED: u32 = 1 << 10;
    pub const BOND_TRIGGERED: u32 = 1 << 11;
    pub const HERO_MESSAGE: u32 = 1 << 12;
    pub const SWORD_BOOST: u32 = 1 << 13;
    pub const SHIELD_BOOST: u32 = 1 << 14;
    pub const SYRUP_TRIGGERED: u32 = 1 << 15;
    pub const TERA_BLOCKED: u32 = 1 << 16;
    pub const FORME_REGRESSION: u32 = 1 << 17;
    pub const MAYBE_TRAPPED: u32 = 1 << 18;
    pub const MAYBE_DISABLED: u32 = 1 << 19;
    pub const MAYBE_LOCKED: u32 = 1 << 20;
    pub const SWITCH_REQUESTED: u32 = 1 << 21;
}
impl Default for Pokemon {
    fn default() -> Self {
        Self {
            flags: 0,
            stellar_boosted_types: 0,
            stored_stats: [0; 5],
            base_stored_stats: [0; 5],
            base_move_slots: [MoveSlot::default(); 4],
            virtual_move_slots: [MoveSlot::default(); 4],
            attacks: [AttackRecord::default(); 12],
            volatiles: EffectList::default(),
            species: EffectId::NONE,
            base_species: EffectId::NONE,
            ability: EffectId::NONE,
            base_ability: EffectId::NONE,
            item: EffectId::NONE,
            last_item: EffectId::NONE,
            status_state: CellId::NONE,
            ability_state: CellId::NONE,
            item_state: CellId::NONE,
            species_state: CellId::NONE,
            last_move: EffectId::NONE,
            switch_flag: EffectId::NONE,
            hp: 0,
            max_hp: 0,
            weighthg: 0,
            speed: 0,
            active_turns: 0,
            active_move_actions: 0,
            times_attacked: 0,
            types: [TypeId::NONE; 2],
            apparent_types: [TypeId::NONE; 2],
            boosts: [0; 7],
            added_type: TypeId::NONE,
            terastallized: TypeId::NONE,
            position: 0,
            illusion: MonId::NONE,
            move_count: 0,
            virtual_move_count: 0,
            last_move_target_loc: 0,
            status: Status::None,
            trapped: Trapped::No,
            move_this_turn_result: ResultFlag::Undefined,
            move_last_turn_result: ResultFlag::Undefined,
            show_cure: ResultFlag::Undefined,
        }
    }
}
impl Pokemon {
    /// Ordinary slots alias baseMoveSlots in JS; overlay slots exist only after Transform.
    pub fn move_slots(&self) -> &[MoveSlot] {
        if self.flags & mon_flags::TRANSFORMED != 0 {
            &self.virtual_move_slots[..self.virtual_move_count as usize]
        } else {
            &self.base_move_slots[..self.move_count as usize]
        }
    }
}
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Side {
    pub party: [MonId; 6],
    pub active: [MonId; 2],
    pub conditions: EffectList<9>,
    pub slot_conditions: [EffectList<2>; 2],
    pub choice: choices::SideChoice,
    pub pokemon_count: u8,
    pub pokemon_left: u8,
    pub total_fainted: u16,
    pub tera_used: bool,
    pub fainted_this_turn: ResultFlag,
    pub fainted_last_turn: ResultFlag,
}
impl Default for Side {
    fn default() -> Self {
        Self {
            party: [MonId::NONE; 6],
            active: [MonId::NONE; 2],
            conditions: EffectList::default(),
            slot_conditions: [EffectList::default(); 2],
            choice: choices::SideChoice::default(),
            pokemon_count: 0,
            pokemon_left: 0,
            total_fainted: 0,
            tera_used: false,
            fainted_this_turn: ResultFlag::Null,
            fainted_last_turn: ResultFlag::Null,
        }
    }
}
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Field {
    pub weather: CellId,
    pub terrain: CellId,
    pub pseudo_weather: EffectList<14>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
    #[default]
    Created,
    Start,
    Choices,
    Running,
    Ended,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct FaintEntry {
    pub target: MonId,
    pub source: MonId,
    pub effect: EffectToken,
}
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct BattleState {
    /// Conservative protocol/switch history; independent of Pokemon's mechanic flag bits.
    pub revealed_mons: u16,
    pub prng: Prng,
    pub pokemon: [Pokemon; 12],
    pub sides: [Side; 2],
    pub field: Field,
    pub effects: EffectArena,
    pub queue: choices::ActionQueue,
    pub requests: [choices::SideRequest; 2],
    pub faint_queue: [FaintEntry; 12],
    pub faint_queue_len: u8,
    pub effect_order: u32,
    pub attack_seq: u32,
    pub seq_at_last_end_turn: u32,
    pub turn: u16,
    pub phase: Phase,
    pub request_state: choices::RequestKind,
    pub mid_turn: bool,
    pub started: bool,
    pub ended: bool,
    pub winner: SideId,
    pub speed_order: [u8; 4],
    pub last_move: EffectId,
    pub last_successful_move: EffectId,
    pub format_state: CellId,
    pub support_cancel: bool,
    pub report_percentages: bool,
    pub illusion_hint: bool,
}
impl BattleState {
    pub fn empty(seed: [u16; 4]) -> Self {
        let mut effects = EffectArena::default();
        let weather = effects.alloc(Holder::FIELD, Holder::NONE, EffectId::NONE, 0);
        let terrain = effects.alloc(Holder::FIELD, Holder::NONE, EffectId::NONE, 0);
        let format_state = effects.alloc(Holder::BATTLE, Holder::NONE, EffectId::NONE, 0);
        Self {
            revealed_mons: 0,
            prng: Prng::from_seed(seed),
            pokemon: [Pokemon::default(); 12],
            sides: [Side::default(); 2],
            field: Field {
                weather,
                terrain,
                pseudo_weather: EffectList::default(),
            },
            effects,
            queue: choices::ActionQueue::default(),
            requests: [choices::SideRequest::default(); 2],
            faint_queue: [FaintEntry::default(); 12],
            faint_queue_len: 0,
            effect_order: 0,
            attack_seq: 0,
            seq_at_last_end_turn: 0,
            turn: 0,
            phase: Phase::Created,
            request_state: choices::RequestKind::None,
            mid_turn: false,
            started: false,
            ended: false,
            winner: SideId(255),
            speed_order: [0, 1, 2, 3],
            last_move: EffectId::NONE,
            last_successful_move: EffectId::NONE,
            format_state,
            support_cancel: false,
            report_percentages: false,
            illusion_hint: false,
        }
    }
}
const _: () = assert!(core::mem::size_of::<BattleState>() <= 40 * 1024);

impl BattleState {
    pub fn record_attack(
        &mut self,
        target: MonId,
        source: MonId,
        move_id: EffectId,
        damage: Option<u32>,
    ) {
        assert_ne!(target, source, "Self-attacks are not added to attackedBy");
        self.attack_seq = self
            .attack_seq
            .checked_add(1)
            .expect("Attack sequence overflow");
        let slot = SlotId::new(source.side(), self.pokemon[source.0 as usize].position);
        let r = &mut self.pokemon[target.0 as usize].attacks[source.0 as usize];
        r.last_seq = self.attack_seq;
        r.last_damage = damage.unwrap_or(0);
        r.last_slot = slot;
        r.last_move = move_id;
        r.flags |= 1;
        if let Some(damage) = damage {
            r.numeric_seq = self.attack_seq;
            r.numeric_damage = damage;
            r.numeric_slot = slot;
            r.flags |= 2;
            if damage > 0 {
                r.flags |= 4
            }
        }
    }
    pub fn last_attacked_by(&self, target: MonId) -> Option<(MonId, &AttackRecord)> {
        self.pokemon[target.0 as usize]
            .attacks
            .iter()
            .enumerate()
            .filter(|(_, r)| r.flags & 1 != 0)
            .max_by_key(|(_, r)| r.last_seq)
            .map(|(s, r)| (MonId(s as u8), r))
    }
    /// Showdown tests whether filterOutSameSide is undefined, rather than its
    /// boolean value: Some(false) and Some(true) both exclude allied sources.
    pub fn last_damaged_by(
        &self,
        target: MonId,
        filter_out_same_side: Option<bool>,
    ) -> Option<(MonId, &AttackRecord)> {
        self.pokemon[target.0 as usize]
            .attacks
            .iter()
            .enumerate()
            .filter(|(s, r)| {
                r.flags & 2 != 0
                    && (filter_out_same_side.is_none() || MonId(*s as u8).side() != target.side())
            })
            .max_by_key(|(_, r)| r.numeric_seq)
            .map(|(s, r)| (MonId(s as u8), r))
    }
    pub fn prune_attacks(&mut self, target: MonId) {
        let active_mask = self.pokemon.iter().enumerate().fold(0u16, |mask, (s, p)| {
            mask | if p.flags & mon_flags::ACTIVE != 0 {
                1 << s
            } else {
                0
            }
        });
        for (s, r) in self.pokemon[target.0 as usize]
            .attacks
            .iter_mut()
            .enumerate()
        {
            if active_mask & (1 << s) == 0 {
                *r = AttackRecord::default()
            } else {
                r.flags &= !4;
            }
        }
    }
}
const _: () = assert!(core::mem::size_of::<BattleState>() == 37688);
const _: () = assert!(core::mem::size_of::<Pokemon>() == 540);
const _: () = assert!(core::mem::size_of::<Side>() == 74);
const _: () = assert!(core::mem::size_of::<FaintEntry>() == 4);
const _: () = assert!(core::mem::size_of::<AttackRecord>() == 24);
const _: () = assert!(core::mem::size_of::<choices::Action>() == 40);
const _: () = assert!(core::mem::size_of::<choices::SideRequest>() == 96);
