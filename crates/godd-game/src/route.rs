//! Route guide (a remaster aid, `hud_route`): one level's way through, recorded from a run that
//! finished it (`playbot --record`), shown as a beacon at the next thing to do and a trail of
//! lights to it. Routes are made on the player's machine from their own game files and loaded
//! from `routes/` folders; none ship with the game.
//!
//! File format (text, one item per line, `#` comments):
//! ```text
//! route 1
//! step wall 112 14784 16056 640
//! t 14700 16000 640
//! step object 257 15024 16976 672
//! ```
//! A `step` names the wall switch, object or trigger region used, and where; the `t` lines
//! after it are the trail walked from it to the next step.

/// What a step uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteTarget {
    Wall(usize),
    Object(usize),
    Region(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteStep {
    pub target: RouteTarget,
    pub pos: [f32; 3],
    /// The way walked from the previous step to this one.
    pub trail: Vec<[f32; 3]>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Route {
    pub steps: Vec<RouteStep>,
}

impl Route {
    /// Parse a route file; None if it is not one.
    pub fn parse(text: &str) -> Option<Route> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'));
        if !lines.next()?.starts_with("route") {
            return None;
        }
        let mut route = Route::default();
        let mut trail: Vec<[f32; 3]> = Vec::new();
        for line in lines {
            let mut it = line.split_whitespace();
            let num = |s: Option<&str>| s.and_then(|v| v.parse::<f32>().ok());
            match it.next() {
                Some("t") => {
                    if let (Some(x), Some(y), Some(z)) = (num(it.next()), num(it.next()), num(it.next())) {
                        trail.push([x, y, z]);
                    }
                }
                Some("step") => {
                    let kind = it.next();
                    let id = it.next().and_then(|v| v.parse::<usize>().ok());
                    let (Some(x), Some(y), Some(z)) = (num(it.next()), num(it.next()), num(it.next())) else { continue };
                    let target = match (kind, id) {
                        (Some("wall"), Some(i)) => RouteTarget::Wall(i),
                        (Some("object"), Some(i)) => RouteTarget::Object(i),
                        (Some("region"), Some(i)) => RouteTarget::Region(i),
                        _ => continue,
                    };
                    route.steps.push(RouteStep { target, pos: [x, y, z], trail: std::mem::take(&mut trail) });
                }
                _ => {}
            }
        }
        (!route.steps.is_empty()).then_some(route)
    }

    /// The file text (see the module doc).
    pub fn to_text(&self) -> String {
        let mut out = String::from("route 1\n");
        for step in &self.steps {
            for p in &step.trail {
                out.push_str(&format!("t {:.0} {:.0} {:.0}\n", p[0], p[1], p[2]));
            }
            let (kind, id) = match step.target {
                RouteTarget::Wall(i) => ("wall", i),
                RouteTarget::Object(i) => ("object", i),
                RouteTarget::Region(i) => ("region", i),
            };
            out.push_str(&format!("step {kind} {id} {:.0} {:.0} {:.0}\n", step.pos[0], step.pos[1], step.pos[2]));
        }
        out
    }
}

/// A trail without its loops: wherever it comes back within `near` of an earlier point, the
/// stretch in between (a detour, a dead end tried) is cut.
pub fn untangle(trail: &[[f32; 3]], near: f32) -> Vec<[f32; 3]> {
    let mut out: Vec<[f32; 3]> = Vec::new();
    for &p in trail {
        let close = |q: &[f32; 3]| (q[0] - p[0]).hypot(q[1] - p[1]) < near && (q[2] - p[2]).abs() < 64.0;
        match out.iter().position(close) {
            Some(k) => out.truncate(k + 1),
            None => out.push(p),
        }
    }
    out
}

/// How far along a route the player is: `next` is the step to show.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteProgress {
    pub next: usize,
}

/// How many steps ahead a use may skip to (things done out of order).
const LOOKAHEAD: usize = 6;

impl RouteProgress {
    /// Follow what the player did since last time: switch walls that fired, objects bumped,
    /// regions entered. A use of the next step, or of one a few steps on, moves past it.
    pub fn advance(&mut self, route: &Route, walls: &[usize], objects: &[usize], regions: &[usize]) {
        let used = |t: RouteTarget| match t {
            RouteTarget::Wall(w) => walls.contains(&w),
            RouteTarget::Object(o) => objects.contains(&o),
            RouteTarget::Region(r) => regions.contains(&r),
        };
        let end = (self.next + LOOKAHEAD).min(route.steps.len());
        if let Some(k) = (self.next..end).rev().find(|&k| used(route.steps[k].target)) {
            self.next = k + 1;
        }
    }

    /// Steps already done in the world (a pickup no longer there): skip them.
    pub fn skip_done(&mut self, route: &Route, gone: impl Fn(usize) -> bool) {
        while let Some(step) = route.steps.get(self.next) {
            match step.target {
                RouteTarget::Object(o) if gone(o) => self.next += 1,
                _ => break,
            }
        }
    }

    /// The step to show, if the route is not done.
    pub fn current<'a>(&self, route: &'a Route) -> Option<&'a RouteStep> {
        route.steps.get(self.next)
    }

    /// Up to `max` trail points of the current step ahead of `at`, `spacing` apart.
    pub fn trail_ahead(&self, route: &Route, at: [f32; 3], spacing: f32, max: usize) -> Vec<[f32; 3]> {
        let Some(step) = self.current(route) else { return Vec::new() };
        let trail = &step.trail;
        if trail.is_empty() {
            return Vec::new();
        }
        let d = |p: &[f32; 3]| (p[0] - at[0]).hypot(p[1] - at[1]);
        let start = (0..trail.len()).min_by(|&a, &b| d(&trail[a]).total_cmp(&d(&trail[b]))).unwrap_or(0);
        let mut out = Vec::new();
        let mut last = at;
        for p in trail[start..].iter().chain(std::iter::once(&step.pos)) {
            if (p[0] - last[0]).hypot(p[1] - last[1]) >= spacing {
                out.push(*p);
                last = *p;
                if out.len() >= max {
                    break;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Route {
        Route {
            steps: vec![
                RouteStep { target: RouteTarget::Wall(112), pos: [100.0, 0.0, 0.0], trail: vec![[0.0, 0.0, 0.0], [50.0, 0.0, 0.0]] },
                RouteStep { target: RouteTarget::Object(257), pos: [200.0, 0.0, 0.0], trail: vec![[150.0, 0.0, 0.0]] },
                RouteStep { target: RouteTarget::Region(9), pos: [300.0, 0.0, 0.0], trail: vec![] },
            ],
        }
    }

    #[test]
    fn text_round_trip() {
        let r = sample();
        assert_eq!(Route::parse(&r.to_text()), Some(r));
        assert_eq!(Route::parse("not a route"), None);
    }

    #[test]
    fn progress_follows_uses_and_skips_ahead() {
        let r = sample();
        let mut p = RouteProgress::default();
        p.advance(&r, &[5], &[], &[]);
        assert_eq!(p.next, 0, "an unrelated switch");
        p.advance(&r, &[112], &[], &[]);
        assert_eq!(p.next, 1);
        // Done out of order: the region before the pickup.
        p.advance(&r, &[], &[], &[9]);
        assert_eq!(p.next, 3);
        assert!(p.current(&r).is_none());
        let mut q = RouteProgress { next: 1 };
        q.skip_done(&r, |o| o == 257);
        assert_eq!(q.next, 2);
    }

    #[test]
    fn untangle_cuts_detours() {
        let t = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [200.0, 0.0, 0.0], [210.0, 100.0, 0.0], [105.0, 5.0, 0.0], [100.0, 200.0, 0.0]];
        assert_eq!(untangle(&t, 32.0), vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 200.0, 0.0]], "back past (100, 0): the loop out to 200 is cut");
    }

    #[test]
    fn trail_ahead_starts_near_the_player() {
        let r = sample();
        let p = RouteProgress::default();
        let ahead = p.trail_ahead(&r, [40.0, 0.0, 0.0], 20.0, 4);
        assert_eq!(ahead, vec![[100.0, 0.0, 0.0]], "past the nearest trail point, on to the step");
    }
}
