//! Game-agnostic core shared by every game module and front end: the software frame, the
//! virtual file system with mod overlays, the fixed-rate clock and audio requests. No platform
//! dependencies; builds for WebAssembly.

pub mod audio;
pub mod clock;
pub mod frame;
pub mod vfs;

pub use audio::{AudioEvent, wav_seconds};
pub use clock::FixedClock;
pub use frame::Frame;
pub use vfs::Vfs;
