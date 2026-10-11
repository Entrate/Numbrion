//! STUB (replaced by the real pool on merge).
use super::SplitMix64;
use crate::{ids::*, teams::{Gender, SetDef}};
pub struct SetConstraints<'a> { pub name: &'a str, pub shown_species: &'a str, pub level: Option<u16>, pub gender: Gender, pub shiny: bool, pub moves: &'a [EffectId], pub item: Option<EffectId>, pub ability: Option<EffectId>, pub tera: Option<TypeId> }
pub struct Sampled { pub set: SetDef, pub consistent: bool }
#[derive(Default)]
pub struct SetPool;
impl SetPool {
    pub fn sample(&self, _c: &SetConstraints, _rng: &mut SplitMix64) -> Option<Sampled> { None }
    pub fn sample_unrevealed(&self, _exclude: &[EffectId], _rng: &mut SplitMix64) -> Option<SetDef> { None }
}
