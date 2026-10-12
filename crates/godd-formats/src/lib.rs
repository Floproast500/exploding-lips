//! Parsers for the data files of the G.O.D.D. engine as shipped with Exploding Lips.
//!
//! Format notes live in `re/formats/`. Every parser works on byte slices so it can be used
//! unchanged from native and WASM front ends.

mod error;
mod reader;

pub mod bmp;
pub mod gam;
pub mod ini;
pub mod pal;
pub mod pic;
pub mod xrb;

pub use error::{Error, Result};
