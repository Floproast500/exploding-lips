//! Game-agnostic scene data for the GPU renderer. Game modules turn their worlds into these
//! shapes; the renderer draws them. Today: sector outlines and triangulation ([`polygon`]).

pub mod polygon;

/// Reusable triangulator state.
pub type Earcut = earcut::Earcut<f64>;
