//! STUB (replaced by the real tracker on merge).
use crate::{ids::*, teams::Gender};
pub enum ItemKnowledge { Unknown, Held(EffectId), Lost }
pub struct MonInfo { pub name: String, pub species: EffectId, pub details_species: String, pub level: u16, pub gender: Gender, pub shiny: bool, pub moves: Vec<EffectId>, pub starting_item: Option<EffectId>, pub starting_ability: Option<EffectId>, pub tera_type: Option<TypeId> }
#[derive(Default)]
pub struct RevealTracker { mons: [Vec<MonInfo>; 2] }
impl RevealTracker {
    pub fn new() -> Self { Self::default() }
    pub fn feed(&mut self, _line: &str) {}
    pub fn mons(&self, side: SideId) -> &[MonInfo] { &self.mons[side.0 as usize] }
    pub fn find(&self, _side: SideId, _name: &str) -> Option<usize> { None }
    pub fn leads(&self, _side: SideId) -> [Option<usize>; 2] { [None; 2] }
    pub fn team_size(&self, _side: SideId) -> Option<u8> { None }
}
