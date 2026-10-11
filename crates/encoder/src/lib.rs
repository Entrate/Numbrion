//! The training observation pipeline in Rust, a port of `crates/pyengine/training`:
//!
//! * [`encode`]: the token encoder (`encoder.py`): a per-row [`tracker::Tracker`] of the player-view protocol plus the
//!   side's own request JSON become the `ids` / `floats` / `field` / `foe_match` / `fainted` / `guard` arrays,
//! * [`damage`]: the damage-calc features (`damage.py`), `action_features` and `token_features`,
//! * [`smart`]: the scripted doubles player (`smart.py`).
//!
//! Static game data comes from `data/training/dex.json` ([`dex::Dex`], `dex.py`). The array layout ([`layout`]) is
//! the Python one, so the Python model and training code read the arrays unchanged. The Python modules stay the
//! reference implementation; `crates/pyengine/training_tests/test_rust_parity.py` compares the two.
//! No dependencies: the Python bindings live in `pyengine` (`py_encoder.rs`).

pub mod damage;
pub mod dex;
pub mod encode;
pub mod json;
pub mod layout;
pub mod smart;
pub mod tracker;

pub use dex::{Dex, to_id};
