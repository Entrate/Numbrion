//! Observation layout shared with crates/pyengine/training/encoder.py (generated from its constant lists;
//! `training_tests/test_rust_parity.py` checks the names and indices against the Python module).

pub const N_TOKENS: usize = 12;
pub const OWN: usize = 0;
pub const FOE_A: usize = 6;
pub const FOE_B: usize = 7;
pub const K_MOVES: usize = 10;
pub const K_ABILITIES: usize = 4;
pub const K_ITEMS: usize = 4;
pub const N_DYNAMIC: usize = 46;
pub const N_POKEMON_FLOATS: usize = 119;
pub const N_FIELD_FLOATS: usize = 52;
pub const I_SPECIES: usize = 0;
pub const I_ITEM: usize = 1;
pub const I_ABILITY: usize = 2;
pub const I_TERA: usize = 3;
pub const I_TYPE1: usize = 4;
pub const I_TYPE2: usize = 5;
pub const I_MOVE0: usize = 6;
pub const I_CAND_MOVES: usize = 10;
pub const I_CAND_ABILITIES: usize = 20;
pub const I_CAND_ITEMS: usize = 24;
pub const N_IDS: usize = 28;
pub const LEGACY_ACTION_FEATURES: usize = 9;
pub const LEGACY_TOKEN_FEATURES: usize = 6;
/// Widths computed by the frozen Python reference; later columns are Rust-only.
pub const REFERENCE_ACTION_FEATURES: usize = 23;
pub const REFERENCE_TOKEN_FEATURES: usize = 10;
pub const N_ACTION_FEATURES: usize = 35;
pub const N_TOKEN_FEATURES: usize = 21;
pub const CACHE_LIMIT: usize = 50000;
pub const STAT_SCALE: f32 = 400.0;
/// Move codes per active slot with action features (4 move slots x 5 targets x Tera bit).
pub const N_CODES: usize = 40;
pub const N_ACTIONS: usize = 47;
pub const PASS: usize = 46;

/// Per-token float columns (`PF` in encoder.py).
pub mod pf {
    pub const PRESENT: usize = 0;
    pub const OWN: usize = 1;
    pub const ACTIVE_A: usize = 2;
    pub const ACTIVE_B: usize = 3;
    pub const BENCH: usize = 4;
    pub const HP: usize = 5;
    pub const FAINTED: usize = 6;
    pub const BRN: usize = 7;
    pub const PAR: usize = 8;
    pub const SLP: usize = 9;
    pub const FRZ: usize = 10;
    pub const PSN: usize = 11;
    pub const TOX: usize = 12;
    pub const BOOST_ATK: usize = 13;
    pub const BOOST_DEF: usize = 14;
    pub const BOOST_SPA: usize = 15;
    pub const BOOST_SPD: usize = 16;
    pub const BOOST_SPE: usize = 17;
    pub const BOOST_ACCURACY: usize = 18;
    pub const BOOST_EVASION: usize = 19;
    pub const TERASTALLIZED: usize = 20;
    pub const TERA_AVAILABLE: usize = 21;
    pub const SWITCH_TURNS: usize = 22;
    pub const FRESH: usize = 23;
    pub const PROTECT_LAST: usize = 24;
    pub const TRAPPED: usize = 25;
    pub const PERISH: usize = 26;
    pub const STAT_HP: usize = 27;
    pub const SUBSTITUTE: usize = 28;
    pub const CONFUSION: usize = 29;
    pub const TAUNT: usize = 30;
    pub const ENCORE: usize = 31;
    pub const DISABLE: usize = 32;
    pub const YAWN: usize = 33;
    pub const LEECHSEED: usize = 34;
    pub const SALTCURE: usize = 35;
    pub const FOCUSENERGY: usize = 36;
    pub const PARTIALLYTRAPPED: usize = 37;
    pub const PP0: usize = 38;
    pub const PP1: usize = 39;
    pub const PP2: usize = 40;
    pub const PP3: usize = 41;
    pub const DISABLED0: usize = 42;
    pub const DISABLED1: usize = 43;
    pub const DISABLED2: usize = 44;
    pub const DISABLED3: usize = 45;
    pub const LEVEL: usize = 46;
    pub const STAT_ATK: usize = 47;
    pub const STAT_DEF: usize = 48;
    pub const STAT_SPA: usize = 49;
    pub const STAT_SPD: usize = 50;
    pub const STAT_SPE: usize = 51;
    pub const ITEM_KNOWN: usize = 52;
    pub const ABILITY_KNOWN: usize = 53;
    pub const TERA_KNOWN: usize = 54;
    pub const MOVES_SEEN: usize = 55;
    pub const ITEM_LOST: usize = 56;
    pub const MOVE_KNOWN0: usize = 57;
    pub const MOVE_KNOWN1: usize = 58;
    pub const MOVE_KNOWN2: usize = 59;
    pub const MOVE_KNOWN3: usize = 60;
    pub const CAND_MOVE_P0: usize = 61;
    pub const CAND_MOVE_P1: usize = 62;
    pub const CAND_MOVE_P2: usize = 63;
    pub const CAND_MOVE_P3: usize = 64;
    pub const CAND_MOVE_P4: usize = 65;
    pub const CAND_MOVE_P5: usize = 66;
    pub const CAND_MOVE_P6: usize = 67;
    pub const CAND_MOVE_P7: usize = 68;
    pub const CAND_MOVE_P8: usize = 69;
    pub const CAND_MOVE_P9: usize = 70;
    pub const CAND_ABILITY_P0: usize = 71;
    pub const CAND_ABILITY_P1: usize = 72;
    pub const CAND_ABILITY_P2: usize = 73;
    pub const CAND_ABILITY_P3: usize = 74;
    pub const CAND_ITEM_P0: usize = 75;
    pub const CAND_ITEM_P1: usize = 76;
    pub const CAND_ITEM_P2: usize = 77;
    pub const CAND_ITEM_P3: usize = 78;
    pub const TERA_P0: usize = 79;
    pub const TERA_P1: usize = 80;
    pub const TERA_P2: usize = 81;
    pub const TERA_P3: usize = 82;
    pub const TERA_P4: usize = 83;
    pub const TERA_P5: usize = 84;
    pub const TERA_P6: usize = 85;
    pub const TERA_P7: usize = 86;
    pub const TERA_P8: usize = 87;
    pub const TERA_P9: usize = 88;
    pub const TERA_P10: usize = 89;
    pub const TERA_P11: usize = 90;
    pub const TERA_P12: usize = 91;
    pub const TERA_P13: usize = 92;
    pub const TERA_P14: usize = 93;
    pub const TERA_P15: usize = 94;
    pub const TERA_P16: usize = 95;
    pub const TERA_P17: usize = 96;
    pub const TERA_P18: usize = 97;
    pub const TERA_P19: usize = 98;
    pub const TYPE_MULT0: usize = 99;
    pub const TYPE_MULT1: usize = 100;
    pub const TYPE_MULT2: usize = 101;
    pub const TYPE_MULT3: usize = 102;
    pub const TYPE_MULT4: usize = 103;
    pub const TYPE_MULT5: usize = 104;
    pub const TYPE_MULT6: usize = 105;
    pub const TYPE_MULT7: usize = 106;
    pub const TYPE_MULT8: usize = 107;
    pub const TYPE_MULT9: usize = 108;
    pub const TYPE_MULT10: usize = 109;
    pub const TYPE_MULT11: usize = 110;
    pub const TYPE_MULT12: usize = 111;
    pub const TYPE_MULT13: usize = 112;
    pub const TYPE_MULT14: usize = 113;
    pub const TYPE_MULT15: usize = 114;
    pub const TYPE_MULT16: usize = 115;
    pub const TYPE_MULT17: usize = 116;
    pub const TYPE_MULT18: usize = 117;
    pub const TYPE_MULT19: usize = 118;
}

/// Field float columns (`FF` in encoder.py).
pub mod ff {
    pub const SUN: usize = 0;
    pub const RAIN: usize = 1;
    pub const SAND: usize = 2;
    pub const SNOW: usize = 3;
    pub const EXTREME_WEATHER: usize = 4;
    pub const WEATHER_TURNS: usize = 5;
    pub const ELECTRICTERRAIN: usize = 6;
    pub const GRASSYTERRAIN: usize = 7;
    pub const MISTYTERRAIN: usize = 8;
    pub const PSYCHICTERRAIN: usize = 9;
    pub const TERRAIN_TURNS: usize = 10;
    pub const TRICKROOM: usize = 11;
    pub const TRICKROOM_TURNS: usize = 12;
    pub const GRAVITY: usize = 13;
    pub const OWN_TAILWIND: usize = 14;
    pub const OWN_REFLECT: usize = 15;
    pub const OWN_LIGHTSCREEN: usize = 16;
    pub const OWN_AURORAVEIL: usize = 17;
    pub const OWN_STEALTHROCK: usize = 18;
    pub const OWN_SPIKES: usize = 19;
    pub const OWN_TOXICSPIKES: usize = 20;
    pub const OWN_STICKYWEB: usize = 21;
    pub const OWN_SAFEGUARD: usize = 22;
    pub const OWN_MIST: usize = 23;
    pub const OWN_TAILWIND_TURNS: usize = 24;
    pub const OWN_REFLECT_TURNS: usize = 25;
    pub const OWN_LIGHTSCREEN_TURNS: usize = 26;
    pub const OWN_AURORAVEIL_TURNS: usize = 27;
    pub const FOE_TAILWIND: usize = 28;
    pub const FOE_REFLECT: usize = 29;
    pub const FOE_LIGHTSCREEN: usize = 30;
    pub const FOE_AURORAVEIL: usize = 31;
    pub const FOE_STEALTHROCK: usize = 32;
    pub const FOE_SPIKES: usize = 33;
    pub const FOE_TOXICSPIKES: usize = 34;
    pub const FOE_STICKYWEB: usize = 35;
    pub const FOE_SAFEGUARD: usize = 36;
    pub const FOE_MIST: usize = 37;
    pub const FOE_TAILWIND_TURNS: usize = 38;
    pub const FOE_REFLECT_TURNS: usize = 39;
    pub const FOE_LIGHTSCREEN_TURNS: usize = 40;
    pub const FOE_AURORAVEIL_TURNS: usize = 41;
    pub const TURN: usize = 42;
    pub const OWN_TERA_USED: usize = 43;
    pub const FOE_TERA_USED: usize = 44;
    pub const REQUEST_MOVE: usize = 45;
    pub const REQUEST_SWITCH: usize = 46;
    pub const REQUEST_WAIT: usize = 47;
    pub const OWN_ALIVE: usize = 48;
    pub const FOE_FAINTED: usize = 49;
    pub const FOE_REVEALED: usize = 50;
    pub const FOE_UNREVEALED: usize = 51;
}

pub const POKEMON_FLOAT_NAMES: [&str; 119] = [
    "present",
    "own",
    "active_a",
    "active_b",
    "bench",
    "hp",
    "fainted",
    "brn",
    "par",
    "slp",
    "frz",
    "psn",
    "tox",
    "boost_atk",
    "boost_def",
    "boost_spa",
    "boost_spd",
    "boost_spe",
    "boost_accuracy",
    "boost_evasion",
    "terastallized",
    "tera_available",
    "switch_turns",
    "fresh",
    "protect_last",
    "trapped",
    "perish",
    "stat_hp",
    "substitute",
    "confusion",
    "taunt",
    "encore",
    "disable",
    "yawn",
    "leechseed",
    "saltcure",
    "focusenergy",
    "partiallytrapped",
    "pp0",
    "pp1",
    "pp2",
    "pp3",
    "disabled0",
    "disabled1",
    "disabled2",
    "disabled3",
    "level",
    "stat_atk",
    "stat_def",
    "stat_spa",
    "stat_spd",
    "stat_spe",
    "item_known",
    "ability_known",
    "tera_known",
    "moves_seen",
    "item_lost",
    "move_known0",
    "move_known1",
    "move_known2",
    "move_known3",
    "cand_move_p0",
    "cand_move_p1",
    "cand_move_p2",
    "cand_move_p3",
    "cand_move_p4",
    "cand_move_p5",
    "cand_move_p6",
    "cand_move_p7",
    "cand_move_p8",
    "cand_move_p9",
    "cand_ability_p0",
    "cand_ability_p1",
    "cand_ability_p2",
    "cand_ability_p3",
    "cand_item_p0",
    "cand_item_p1",
    "cand_item_p2",
    "cand_item_p3",
    "tera_p0",
    "tera_p1",
    "tera_p2",
    "tera_p3",
    "tera_p4",
    "tera_p5",
    "tera_p6",
    "tera_p7",
    "tera_p8",
    "tera_p9",
    "tera_p10",
    "tera_p11",
    "tera_p12",
    "tera_p13",
    "tera_p14",
    "tera_p15",
    "tera_p16",
    "tera_p17",
    "tera_p18",
    "tera_p19",
    "type_mult0",
    "type_mult1",
    "type_mult2",
    "type_mult3",
    "type_mult4",
    "type_mult5",
    "type_mult6",
    "type_mult7",
    "type_mult8",
    "type_mult9",
    "type_mult10",
    "type_mult11",
    "type_mult12",
    "type_mult13",
    "type_mult14",
    "type_mult15",
    "type_mult16",
    "type_mult17",
    "type_mult18",
    "type_mult19",
];

pub const FIELD_FLOAT_NAMES: [&str; 52] = [
    "sun",
    "rain",
    "sand",
    "snow",
    "extreme_weather",
    "weather_turns",
    "electricterrain",
    "grassyterrain",
    "mistyterrain",
    "psychicterrain",
    "terrain_turns",
    "trickroom",
    "trickroom_turns",
    "gravity",
    "own_tailwind",
    "own_reflect",
    "own_lightscreen",
    "own_auroraveil",
    "own_stealthrock",
    "own_spikes",
    "own_toxicspikes",
    "own_stickyweb",
    "own_safeguard",
    "own_mist",
    "own_tailwind_turns",
    "own_reflect_turns",
    "own_lightscreen_turns",
    "own_auroraveil_turns",
    "foe_tailwind",
    "foe_reflect",
    "foe_lightscreen",
    "foe_auroraveil",
    "foe_stealthrock",
    "foe_spikes",
    "foe_toxicspikes",
    "foe_stickyweb",
    "foe_safeguard",
    "foe_mist",
    "foe_tailwind_turns",
    "foe_reflect_turns",
    "foe_lightscreen_turns",
    "foe_auroraveil_turns",
    "turn",
    "own_tera_used",
    "foe_tera_used",
    "request_move",
    "request_switch",
    "request_wait",
    "own_alive",
    "foe_fainted",
    "foe_revealed",
    "foe_unrevealed",
];

pub const VOLATILES: [&str; 10] = ["substitute", "confusion", "taunt", "encore", "disable", "yawn", "leechseed", "saltcure", "focusenergy", "partiallytrapped"];
pub const TERRAINS: [&str; 4] = ["electricterrain", "grassyterrain", "mistyterrain", "psychicterrain"];
pub const SIDE_CONDITIONS: [&str; 10] = ["tailwind", "reflect", "lightscreen", "auroraveil", "stealthrock", "spikes", "toxicspikes", "stickyweb", "safeguard", "mist"];
pub const TIMED_SIDE: [&str; 4] = ["tailwind", "reflect", "lightscreen", "auroraveil"];
pub const EXTREME_WEATHER: [&str; 3] = ["desolateland", "primordialsea", "deltastream"];
pub const STATUSES: [&str; 6] = ["brn", "par", "slp", "frz", "psn", "tox"];
pub const BOOSTS: [&str; 7] = ["atk", "def", "spa", "spd", "spe", "accuracy", "evasion"];
/// Weather id -> field column (`WEATHERS`); Delta Stream's 4 is the extreme_weather column.
pub fn weather_column(id: &str) -> Option<usize> {
    match id {
        "sunnyday" => Some(0),
        "desolateland" => Some(0),
        "raindance" => Some(1),
        "primordialsea" => Some(1),
        "sandstorm" => Some(2),
        "snowscape" => Some(3),
        "snow" => Some(3),
        "hail" => Some(3),
        "deltastream" => Some(4),
        _ => None,
    }
}
/// `PERISH`: perishN volatile -> remaining count.
pub fn perish_count(id: &str) -> Option<i32> {
    match id {
        "perish3" => Some(3),
        "perish2" => Some(2),
        "perish1" => Some(1),
        "perish0" => Some(0),
        _ => None,
    }
}
