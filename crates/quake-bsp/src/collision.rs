//! Collision against a version 29 BSP, from the public file format: point contents and swept
//! traces through the world model's hulls. Hull 0 is the visible geometry (a point), hulls 1
//! and up are the clip trees compiled for box sizes (1: the 32x32x56 player box with its
//! origin 24 units above its feet).

use crate::{Error, i32_at, u16_at};

/// Leaf contents.
pub const EMPTY: i32 = -1;
pub const SOLID: i32 = -2;
pub const WATER: i32 = -3;
pub const SLIME: i32 = -4;
pub const LAVA: i32 = -5;
pub const SKY: i32 = -6;

/// Traces stop this far in front of the surface they hit, so the end point is never inside it.
const EPSILON: f32 = 0.03125;

#[derive(Debug, Clone, Copy)]
struct Plane {
    normal: [f32; 3],
    dist: f32,
}

/// A tree node: plane and the two children (front, back). Hull 0 children: a node index, or
/// a leaf as -(leaf + 1). Clip-hull children: a node index, or contents (negative).
#[derive(Debug, Clone, Copy)]
struct Node {
    plane: u32,
    children: [i32; 2],
}

/// The world model's collision data.
#[derive(Debug, Clone, Default)]
pub struct Hulls {
    planes: Vec<Plane>,
    nodes: Vec<Node>,
    leaf_contents: Vec<i32>,
    clipnodes: Vec<Node>,
    /// Head node of each hull for the world model (hull 0 into `nodes`, the rest into
    /// `clipnodes`).
    heads: Vec<i32>,
}

/// The result of a swept trace.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trace {
    /// How far along the move the trace got (1 = all the way).
    pub fraction: f32,
    pub end: [f32; 3],
    /// Normal of the surface hit (zero when nothing was hit).
    pub normal: [f32; 3],
    /// The start point was inside solid.
    pub start_solid: bool,
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

impl Hulls {
    /// Read the collision lumps; `model_record` is the model record size (64 Quake, 80 Hexen II).
    pub(crate) fn parse<'a>(lump: &dyn Fn(usize) -> Result<&'a [u8], Error>, model_record: usize) -> Result<Self, Error> {
        const PLANES: usize = 1;
        const NODES: usize = 5;
        const CLIPNODES: usize = 9;
        const LEAFS: usize = 10;
        const MODELS: usize = 14;
        let f = |c: &[u8], o: usize| f32::from_bits(i32_at(c, o).unwrap_or(0) as u32);
        let planes = lump(PLANES)?.as_chunks::<20>().0.iter().map(|c| Plane { normal: [f(c, 0), f(c, 4), f(c, 8)], dist: f(c, 12) }).collect();
        let child = |c: &[u8], o: usize| u16_at(c, o).unwrap_or(0) as i16 as i32;
        let nodes = lump(NODES)?.as_chunks::<24>().0.iter().map(|c| Node { plane: i32_at(c, 0).unwrap_or(0) as u32, children: [child(c, 4), child(c, 6)] }).collect();
        let clipnodes = lump(CLIPNODES)?.as_chunks::<8>().0.iter().map(|c| Node { plane: i32_at(c, 0).unwrap_or(0) as u32, children: [child(c, 4), child(c, 6)] }).collect();
        let leaf_contents = lump(LEAFS)?.as_chunks::<28>().0.iter().map(|c| i32_at(c, 0).unwrap_or(SOLID)).collect();
        let hulls = (model_record - 40) / 4; // after mins, maxs, origin: head nodes, then 3 ints
        let world = lump(MODELS)?.get(..model_record).ok_or_else(|| Error("no world model".into()))?;
        let heads = (0..hulls).map(|h| i32_at(world, 36 + 4 * h).unwrap_or(0)).collect();
        Ok(Self { planes, nodes, leaf_contents, clipnodes, heads })
    }

    pub fn hull_count(&self) -> usize {
        self.heads.len()
    }

    /// Contents of `p` in `hull` (hull 0: the point itself; hull 1: the player box at that
    /// origin).
    pub fn contents(&self, hull: usize, p: [f32; 3]) -> i32 {
        let Some(&head) = self.heads.get(hull) else { return SOLID };
        let mut n = head;
        for _ in 0..4096 {
            let (tree, leaf_of) = self.tree(hull);
            if n < 0 {
                return leaf_of(n);
            }
            let Some(node) = tree.get(n as usize) else { return SOLID };
            let plane = self.planes[node.plane as usize];
            n = node.children[usize::from(dot(plane.normal, p) - plane.dist < 0.0)];
        }
        SOLID
    }

    fn tree(&self, hull: usize) -> (&[Node], impl Fn(i32) -> i32 + '_) {
        let nodes = if hull == 0 { &self.nodes[..] } else { &self.clipnodes[..] };
        (nodes, move |n: i32| if hull == 0 { self.leaf_contents.get((-n - 1) as usize).copied().unwrap_or(SOLID) } else { n })
    }

    /// Sweep `hull` from `start` to `end`: how far it gets before entering solid (sky counts
    /// as solid, water and lava do not).
    pub fn trace(&self, hull: usize, start: [f32; 3], end: [f32; 3]) -> Trace {
        let solid = |c: i32| c == SOLID || c == SKY;
        if solid(self.contents(hull, start)) {
            return Trace { fraction: 0.0, end: start, normal: [0.0; 3], start_solid: true };
        }
        let mut hit: Option<(f32, [f32; 3])> = None;
        if let Some(&head) = self.heads.get(hull) {
            self.sweep(hull, head, start, end, 0.0, 1.0, start, end, &mut hit, 0);
        }
        match hit {
            Some((fraction, normal)) => Trace { fraction, end: lerp(start, end, fraction), normal, start_solid: false },
            None => Trace { fraction: 1.0, end, normal: [0.0; 3], start_solid: false },
        }
    }

    /// Find where the segment p1 (fraction f1) → p2 (f2) of the full move `start` → `end`
    /// first enters solid below node `n`. Returns true when it did (stop searching).
    #[allow(clippy::too_many_arguments)]
    fn sweep(&self, hull: usize, n: i32, p1: [f32; 3], p2: [f32; 3], f1: f32, f2: f32, start: [f32; 3], end: [f32; 3], hit: &mut Option<(f32, [f32; 3])>, depth: u32) -> bool {
        let (tree, leaf_of) = self.tree(hull);
        if n < 0 {
            let c = leaf_of(n);
            return c == SOLID || c == SKY;
        }
        let Some(node) = tree.get(n as usize).copied() else { return true };
        if depth > 1024 {
            return false;
        }
        let plane = self.planes[node.plane as usize];
        let (d1, d2) = (dot(plane.normal, p1) - plane.dist, dot(plane.normal, p2) - plane.dist);
        if d1 >= 0.0 && d2 >= 0.0 {
            return self.sweep(hull, node.children[0], p1, p2, f1, f2, start, end, hit, depth + 1);
        }
        if d1 < 0.0 && d2 < 0.0 {
            return self.sweep(hull, node.children[1], p1, p2, f1, f2, start, end, hit, depth + 1);
        }
        // The segment crosses the plane: near side first, then the far side from the crossing.
        let near = usize::from(d1 < 0.0);
        let t = (d1 / (d1 - d2)).clamp(0.0, 1.0);
        let (fm, mid) = (f1 + (f2 - f1) * t, lerp(p1, p2, t));
        if self.sweep(hull, node.children[near], p1, mid, f1, fm, start, end, hit, depth + 1) {
            return true;
        }
        let far = node.children[1 - near];
        let far_solid = if far < 0 { matches!(leaf_of(far), SOLID | SKY) } else { false };
        if far_solid || (far >= 0 && matches!(self.contents_from(hull, far, mid), SOLID | SKY)) {
            // Entered solid at the plane: stop just in front of it, facing the mover.
            if hit.is_none() {
                let sign = if near == 0 { 1.0 } else { -1.0 };
                let (s1, s2) = (dot(plane.normal, start) - plane.dist, dot(plane.normal, end) - plane.dist);
                let fraction = if (s1 - s2).abs() > 1e-6 { ((s1 - sign * EPSILON) / (s1 - s2)).clamp(0.0, 1.0) } else { fm };
                *hit = Some((fraction.min(fm), plane.normal.map(|v| v * sign)));
            }
            return true;
        }
        self.sweep(hull, far, mid, p2, fm, f2, start, end, hit, depth + 1)
    }

    /// Contents at `p` starting the descent at node `n`.
    fn contents_from(&self, hull: usize, mut n: i32, p: [f32; 3]) -> i32 {
        let (tree, leaf_of) = self.tree(hull);
        for _ in 0..4096 {
            if n < 0 {
                return leaf_of(n);
            }
            let Some(node) = tree.get(n as usize) else { return SOLID };
            let plane = self.planes[node.plane as usize];
            n = node.children[usize::from(dot(plane.normal, p) - plane.dist < 0.0)];
        }
        SOLID
    }
}
