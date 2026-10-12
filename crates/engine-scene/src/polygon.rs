//! Sector outlines: chain a region's boundary edges into closed loops, sort them into outer
//! boundaries and holes, and triangulate.
//!
//! Edges are directed with the region on their **right** (the usual convention for sector
//! engines whose walls face their front region). Outer boundaries then run clockwise (negative
//! signed area, Y up) and holes (pillars inside the region) counter-clockwise.

use std::collections::HashMap;
use std::f64::consts::TAU;

/// One region's boundary: outer loops with the holes inside each.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outline {
    pub parts: Vec<Part>,
    /// Edges that could not be closed into a loop (malformed data).
    pub open_edges: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    /// Point ids, clockwise.
    pub outer: Vec<u32>,
    /// Point ids, counter-clockwise.
    pub holes: Vec<Vec<u32>>,
}

/// Signed area (counter-clockwise positive).
pub fn signed_area(points: &[[f64; 2]], ring: &[u32]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| {
            let [x0, y0] = points[ring[i] as usize];
            let [x1, y1] = points[ring[(i + 1) % n] as usize];
            x0 * y1 - x1 * y0
        })
        .sum::<f64>()
        / 2.0
}

/// Even-odd point-in-polygon test.
pub fn contains(points: &[[f64; 2]], ring: &[u32], [px, py]: [f64; 2]) -> bool {
    let n = ring.len();
    let mut inside = false;
    for i in 0..n {
        let [x0, y0] = points[ring[i] as usize];
        let [x1, y1] = points[ring[(i + 1) % n] as usize];
        if (y0 > py) != (y1 > py) && px < x0 + (py - y0) * (x1 - x0) / (y1 - y0) {
            inside = !inside;
        }
    }
    inside
}

/// Point ids with identical coordinates mapped to one id (the first), so loops chain through
/// duplicated points.
pub fn weld(points: &[[f64; 2]]) -> Vec<u32> {
    let mut first: HashMap<(u64, u64), u32> = HashMap::new();
    points.iter().enumerate().map(|(i, &[x, y])| *first.entry((x.to_bits(), y.to_bits())).or_insert(i as u32)).collect()
}

/// Chain directed edges (region on the right) into closed loops. At a junction the loop takes
/// the edge that turns furthest right (smallest counter-clockwise angle from the way back), which
/// keeps it hugging the region. Returns the loops and the number of edges left unclosed.
pub fn chain_loops(points: &[[f64; 2]], edges: &[(u32, u32)]) -> (Vec<Vec<u32>>, usize) {
    let mut out: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, &(a, b)) in edges.iter().enumerate() {
        if a != b {
            out.entry(a).or_default().push(i);
        }
    }
    let dir = |from: u32, to: u32| {
        let ([x0, y0], [x1, y1]) = (points[from as usize], points[to as usize]);
        (y1 - y0).atan2(x1 - x0)
    };
    let mut used = vec![false; edges.len()];
    let (mut loops, mut open) = (Vec::new(), 0);
    for start in 0..edges.len() {
        if used[start] || edges[start].0 == edges[start].1 {
            continue;
        }
        let mut ring = Vec::new();
        let mut taken = Vec::new();
        let mut cur = start;
        let closed = loop {
            used[cur] = true;
            taken.push(cur);
            let (u, v) = edges[cur];
            ring.push(u);
            if v == edges[start].0 {
                break true;
            }
            let back = dir(v, u);
            let next = out.get(&v).and_then(|cands| {
                cands
                    .iter()
                    .copied()
                    .filter(|&e| !used[e])
                    .map(|e| {
                        let mut a = (dir(v, edges[e].1) - back).rem_euclid(TAU);
                        if a < 1e-12 {
                            a = TAU; // going straight back is the last resort
                        }
                        (e, a)
                    })
                    .min_by(|x, y| x.1.total_cmp(&y.1))
                    .map(|(e, _)| e)
            });
            match next {
                Some(e) => cur = e,
                None => break false,
            }
        };
        if closed && ring.len() >= 3 {
            loops.push(ring);
        } else {
            open += taken.len();
        }
    }
    (loops, open)
}

/// Sort loops into outer boundaries and the holes inside them. A positive-area loop with no
/// enclosing outer boundary is treated as an outer boundary drawn the wrong way round.
pub fn classify(points: &[[f64; 2]], loops: Vec<Vec<u32>>, open_edges: usize) -> Outline {
    let mut outers: Vec<(Vec<u32>, f64)> = Vec::new();
    let mut holes: Vec<Vec<u32>> = Vec::new();
    for ring in loops {
        let area = signed_area(points, &ring);
        if area < -1e-9 {
            outers.push((ring, -area));
        } else if area > 1e-9 {
            holes.push(ring);
        }
    }
    let mut parts: Vec<Part> = outers.iter().map(|(r, _)| Part { outer: r.clone(), holes: Vec::new() }).collect();
    for hole in holes {
        let probe = interior_probe(points, &hole);
        let parent = (0..outers.len()).filter(|&i| contains(points, &outers[i].0, probe)).min_by(|&a, &b| outers[a].1.total_cmp(&outers[b].1));
        match parent {
            Some(i) => parts[i].holes.push(hole),
            None => {
                let mut outer = hole;
                outer.reverse();
                parts.push(Part { outer, holes: Vec::new() });
            }
        }
    }
    Outline { parts, open_edges }
}

/// A point just inside a hole's boundary (its first edge's midpoint, nudged outward of the
/// hole), used to find the enclosing outer loop even when the hole touches it at a vertex.
fn interior_probe(points: &[[f64; 2]], ring: &[u32]) -> [f64; 2] {
    let [x0, y0] = points[ring[0] as usize];
    let [x1, y1] = points[ring[1] as usize];
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len = (dx * dx + dy * dy).sqrt().max(1e-9);
    // Holes run counter-clockwise, so the region (outside the hole) is on the right.
    [mx + dy / len * 1e-3, my - dx / len * 1e-3]
}

/// Triangulate one part. Triangles are point-id triples, counter-clockwise.
pub fn triangulate(points: &[[f64; 2]], part: &Part, earcut: &mut earcut::Earcut<f64>) -> Vec<[u32; 3]> {
    // Earcut wants the outer ring and holes in opposite orientations; any start works.
    let mut ids: Vec<u32> = part.outer.clone();
    let mut hole_starts = Vec::new();
    for h in &part.holes {
        hole_starts.push(ids.len() as u32);
        ids.extend(h);
    }
    let mut tris: Vec<u32> = Vec::new();
    earcut.earcut(ids.iter().map(|&i| points[i as usize]), &hole_starts, &mut tris);
    tris.chunks_exact(3)
        .map(|t| {
            let tri = [ids[t[0] as usize], ids[t[1] as usize], ids[t[2] as usize]];
            if signed_area(points, &tri) < 0.0 { [tri[0], tri[2], tri[1]] } else { tri }
        })
        .collect()
}

/// Outline and triangles for one region given its directed boundary edges.
pub fn region_mesh(points: &[[f64; 2]], edges: &[(u32, u32)], earcut: &mut earcut::Earcut<f64>) -> (Outline, Vec<[u32; 3]>) {
    let (loops, open) = chain_loops(points, edges);
    let outline = classify(points, loops, open);
    let tris = outline.parts.iter().flat_map(|p| triangulate(points, p, earcut)).collect();
    (outline, tris)
}

/// Net area of an outline (outer areas minus holes).
pub fn outline_area(points: &[[f64; 2]], outline: &Outline) -> f64 {
    outline
        .parts
        .iter()
        .map(|p| -signed_area(points, &p.outer) - p.holes.iter().map(|h| signed_area(points, h)).sum::<f64>())
        .sum()
}

pub fn triangles_area(points: &[[f64; 2]], tris: &[[u32; 3]]) -> f64 {
    tris.iter().map(|t| signed_area(points, t)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Directed edges with the region on the right, from rings given counter-clockwise for the
    /// solid area (outer) and clockwise for holes... i.e. we reverse to make region-on-right.
    fn edges_of(rings: &[&[u32]]) -> Vec<(u32, u32)> {
        rings.iter().flat_map(|r| (0..r.len()).map(move |i| (r[i], r[(i + 1) % r.len()]))).collect()
    }

    #[test]
    fn square_with_hole() {
        // Outer 0..3 clockwise (region on the right), hole 4..7 counter-clockwise.
        let pts = [[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 0.0], [4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]];
        let mut e = edges_of(&[&[0, 1, 2, 3], &[4, 5, 6, 7]]);
        e.reverse(); // edge order must not matter
        let (outline, tris) = region_mesh(&pts, &e, &mut earcut::Earcut::new());
        assert_eq!(outline.parts.len(), 1);
        assert_eq!(outline.parts[0].holes.len(), 1);
        assert_eq!(outline_area(&pts, &outline), 96.0);
        assert!((triangles_area(&pts, &tris) - 96.0).abs() < 1e-9);
        assert!(tris.iter().all(|t| signed_area(&pts, t) > 0.0), "counter-clockwise output");
    }

    #[test]
    fn non_convex_and_two_pieces_touching_at_a_corner() {
        // An L shape and a separate square that shares the L's corner point 2.
        let pts = [[0.0, 0.0], [0.0, 4.0], [2.0, 4.0], [2.0, 2.0], [4.0, 2.0], [4.0, 0.0], [2.0, 6.0], [4.0, 6.0], [4.0, 4.0]];
        let e = edges_of(&[&[0, 1, 2, 3, 4, 5], &[2, 6, 7, 8]]);
        let (outline, tris) = region_mesh(&pts, &e, &mut earcut::Earcut::new());
        assert_eq!(outline.open_edges, 0);
        assert_eq!(outline.parts.len(), 2, "the shared vertex splits into two loops");
        assert_eq!(outline_area(&pts, &outline), 12.0 + 4.0);
        assert!((triangles_area(&pts, &tris) - 16.0).abs() < 1e-9);
    }

    #[test]
    fn duplicated_points_weld_and_open_chains_are_reported() {
        let pts = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        let w = weld(&pts);
        assert_eq!(w[4], 0);
        let e: Vec<(u32, u32)> = [(0, 1), (1, 2), (2, 3), (3, 4)].iter().map(|&(a, b)| (w[a], w[b as usize])).collect();
        let (outline, _) = region_mesh(&pts, &e, &mut earcut::Earcut::new());
        assert_eq!((outline.parts.len(), outline.open_edges), (1, 0));
        let (outline, _) = region_mesh(&pts, &[(0, 1), (1, 2)], &mut earcut::Earcut::new());
        assert_eq!(outline.open_edges, 2);
    }

    #[test]
    fn reversed_lone_loop_becomes_an_outer_boundary() {
        let pts = [[0.0, 0.0], [0.0, 2.0], [2.0, 2.0], [2.0, 0.0]];
        let e = edges_of(&[&[3, 2, 1, 0]]); // counter-clockwise: region on the left
        let (outline, tris) = region_mesh(&pts, &e, &mut earcut::Earcut::new());
        assert_eq!(outline.parts.len(), 1);
        assert!((triangles_area(&pts, &tris) - 4.0).abs() < 1e-9);
    }
}
