//! Showdown's integer/fixed-point helpers (sim/battle.ts `chain`, `chainModify`, `modify`,
//! `finalModify`, `randomizer`; sim/dex.ts `trunc`; lib/utils.ts `clampIntRange`).
//!
//! JS numbers are f64 and `trunc` is `num >>> 0` (ToUint32: truncate toward zero, then wrap
//! mod 2^32). Modifiers are stored as integer 4096ths, which is exact because Showdown always
//! re-truncates `modifier * 4096`.

/// A modifier in 4096ths (`event.modifier * 4096`). 1.0 == 4096.
pub type Mod4096 = u32;
pub const MOD_ONE: Mod4096 = 4096;

/// `trunc(num)` for an integral value: ToUint32.
#[inline(always)]
pub const fn trunc(num: i64) -> u32 {
    num as u32
}

/// `trunc(num)` for a non-integral f64, exactly like JS `num >>> 0`.
#[inline]
pub fn trunc_f64(num: f64) -> u32 {
    if !num.is_finite() {
        return 0;
    }
    let t = num.trunc();
    // ToUint32: modulo 2^32 of the truncated value.
    (t.rem_euclid(4294967296.0)) as u32
}

/// `trunc(num, bits)` with `bits != 0`: `(num >>> 0) % 2**bits`.
#[inline(always)]
pub const fn trunc_bits(num: i64, bits: u32) -> u32 {
    let v = num as u32;
    if bits >= 32 { v } else { v % (1u32 << bits) }
}

/// The 4096ths modifier for `numerator / denominator`: `trunc(numerator * 4096 / denominator)`.
#[inline(always)]
pub fn frac_mod(numerator: u32, denominator: u32) -> Mod4096 {
    trunc_f64(numerator as f64 * 4096.0 / denominator as f64)
}

/// The 4096ths modifier for a float literal used in Showdown, e.g. `chainModify(1.3)` is 5324
/// (not 5325). Always port the literal Showdown passes.
#[inline(always)]
pub fn float_mod(x: f64) -> Mod4096 {
    trunc_f64(x * 4096.0)
}

/// `chain(previousMod, nextMod)` and the core of `chainModify`, on 4096ths.
#[inline(always)]
pub const fn chain(previous: Mod4096, next: Mod4096) -> Mod4096 {
    // ((previousMod * nextMod + 2048) >> 12): JS `>>` works on int32.
    (((previous as i64 * next as i64 + 2048) as i32) >> 12) as u32
}

/// `modify(value, modifier)` where `modifier` is already in 4096ths.
#[inline(always)]
pub fn modify(value: i64, modifier: Mod4096) -> u32 {
    let scaled = trunc(value * modifier as i64) as i64;
    // tr((tr(value * modifier) + 2048 - 1) / 4096): float division then ToUint32.
    trunc_f64((scaled + 2047) as f64 / 4096.0)
}

/// `modify(value, numerator, denominator)`.
#[inline(always)]
pub fn modify_frac(value: i64, numerator: u32, denominator: u32) -> u32 {
    modify(value, frac_mod(numerator, denominator))
}

/// `randomizer(baseDamage)` given the already-drawn `roll = random(16)`.
#[inline(always)]
pub fn randomizer(base_damage: u32, roll: u32) -> u32 {
    let scaled = trunc(base_damage as i64 * (100 - roll as i64)) as i64;
    trunc_f64(scaled as f64 / 100.0)
}

/// `clampIntRange(num, min, max)` for integral input.
#[inline(always)]
pub const fn clamp_int(num: i64, min: i64, max: i64) -> i64 {
    if num < min {
        min
    } else if num > max {
        max
    } else {
        num
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_modifiers_match_js() {
        assert_eq!(float_mod(1.3), 5324);
        assert_eq!(float_mod(1.5), 6144);
        assert_eq!(float_mod(0.75), 3072);
        assert_eq!(float_mod(1.2), 4915);
        assert_eq!(frac_mod(5325, 4096), 5325);
        assert_eq!(frac_mod(2, 3), 2730);
    }

    #[test]
    fn chain_and_modify_match_js() {
        // chain(1.5, 1.5) -> ((6144*6144+2048)>>12) = 9216
        assert_eq!(chain(6144, 6144), 9216);
        // modify(100, 1.5) = tr((tr(100*6144)+2047)/4096) = 150
        assert_eq!(modify(100, 6144), 150);
        // modify(57, 0.75) = tr((tr(57*3072)+2047)/4096) = tr(177151/4096) = 43
        assert_eq!(modify(57, 3072), 43);
        assert_eq!(randomizer(100, 15), 85);
        assert_eq!(randomizer(1, 15), 0);
    }
}
