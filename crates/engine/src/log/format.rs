//! OWNER T: formatting only at the enabled protocol/request output boundary.
#![allow(unused_variables)]
use super::*;
/// Ports pokemon.ts:531-552. PRNG: none. Illusion and active/bench ident semantics.
pub fn write_ident(out: &mut String, view: LogView<'_>, mon: MonId) {
    todo!("stage T: write_ident")
}
/// Ports sim/pokemon.ts:2060-2107; data/rulesets.ts:1353-1360 (HP Percentage Mod).
/// PRNG: none. Secret is exact HP; shared follows the format's HP percentage rule.
pub fn write_health(out: &mut String, view: LogView<'_>, mon: MonId, secret: bool) {
    todo!("stage T: write_health")
}
/// Ports sim/pokemon.ts:524-529,544-552. PRNG: none. Full details respects Illusion.
pub fn write_details(out: &mut String, view: LogView<'_>, mon: MonId, full: bool) {
    todo!("stage T: write_details")
}
/// Ports dex-data.ts:129-143 and dex-moves.ts:477. PRNG: none. Name versus fullname.
pub fn write_effect(out: &mut String, view: LogView<'_>, effect: EffectRef, fullname: bool) {
    todo!("stage T: write_effect")
}
/// Ports battle.ts:3091-3113. PRNG: none. JS join null/undefined preserve empty fields.
pub fn write_arg(out: &mut String, view: LogView<'_>, arg: LogArg<'_>, secret: bool) {
    todo!("stage T: write_arg")
}
