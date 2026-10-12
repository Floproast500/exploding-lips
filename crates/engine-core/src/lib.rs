//! Game-agnostic core shared by every game module and front end: the software frame, the
//! virtual file system with mod overlays, the fixed-rate clock, audio requests, console
//! variables, input bindings and config files. No platform
//! dependencies; builds for WebAssembly.

pub mod audio;
pub mod bind;
pub mod clock;
pub mod config;
pub mod console;
pub mod cvar;
pub mod frame;
pub mod pack;
pub mod vfs;

pub use audio::{AudioEvent, HapticEvent, wav_seconds};
pub use bind::BindTable;
pub use cvar::{Category, CvarRegistry};
pub use clock::FixedClock;
pub use frame::Frame;
pub use vfs::Vfs;
