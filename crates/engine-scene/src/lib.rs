//! Game-agnostic scene data for the GPU renderer. Game modules turn their worlds into these
//! shapes; the renderer draws them: sector outlines and triangulation ([`polygon`]) and the
//! per-frame scene ([`scene`]).

pub mod polygon;
pub mod scene;

/// Reusable triangulator state.
pub type Earcut = earcut::Earcut<f64>;
