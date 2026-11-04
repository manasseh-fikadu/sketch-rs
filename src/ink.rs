//! Freehand ink: input smoothing, pressure simulation, resampling and outline generation.

use crate::geom::{Rect, V2, v2};
use crate::model::StrokePoint;

/// Accumulates pointer samples for a stroke in progress (world coordinates).
#[derive(Debug, Clone)]
pub struct InkBuilder {
    /// 0 = raw input, 1 = maximum smoothing. Controls the streamline filter.
    pub smoothing: f64,
    /// Minimum distance between kept samples, in world units.
    pub min_dist: f64,
    points: Vec<StrokePoint>,
    last_input: Option<(V2, f64)>,
    sim_pressure: f64,
}

impl InkBuilder {
    pub fn new(smoothing: f64, min_dist: f64) -> InkBuilder {
        InkBuilder {
            smoothing: smoothing.clamp(0.0, 1.0),
            min_dist,
            points: Vec::new(),
            last_input: None,
            sim_pressure: 0.5,
        }
    }

    pub fn points(&self) -> &[StrokePoint] {
        &self.points
    }

    /// Add an input sample. `t_ms` is a monotonic timestamp and `speed_scale` converts world
    /// units to screen pixels (the zoom) so pressure simulation is zoom-independent.
    pub fn push(&mut self, pos: V2, pressure: Option<f64>, t_ms: f64, speed_scale: f64) {
        let Some(last) = self.points.last().copied() else {
            self.sim_pressure = pressure.unwrap_or(0.5);
            self.points.push(StrokePoint {
                x: pos.x,
                y: pos.y,
                p: self.sim_pressure,
            });
            self.last_input = Some((pos, t_ms));
            return;
        };
        // Streamline: the pen follows the cursor with exponential lag; more smoothing = more lag.
        let follow = 1.0 - self.smoothing * 0.85;
        let target = last.pos().lerp(pos, follow);
        if target.dist(last.pos()) < self.min_dist {
            return;
        }
        let p = match pressure {
            Some(p) => p.clamp(0.0, 1.0),
            None => {
                let (lp, lt) = self.last_input.unwrap_or((pos, t_ms));
                let dt = (t_ms - lt).max(1.0);
                let speed = lp.dist(pos) * speed_scale / dt; // screen px per ms
                // Fast strokes get thinner, like a real nib.
                let target_p = (1.0 - (speed / 3.0).min(1.0) * 0.75).clamp(0.25, 1.0);
                self.sim_pressure += (target_p - self.sim_pressure) * 0.25;
                self.sim_pressure
            }
        };
        self.last_input = Some((pos, t_ms));
        self.points.push(StrokePoint {
            x: target.x,
            y: target.y,
            p,
        });
    }

    /// Finish the stroke: append the true end point (streamline lags behind) and resample.
    pub fn finish(mut self, end: Option<V2>, spacing: f64) -> Vec<StrokePoint> {
        if let (Some(end), Some(last)) = (end, self.points.last().copied())
            && end.dist(last.pos()) >= self.min_dist
        {
            self.points.push(StrokePoint {
                x: end.x,
                y: end.y,
                p: last.p,
            });
        }
        resample_catmull_rom(&self.points, spacing)
    }
}

/// Resample a polyline through a uniform Catmull-Rom spline at roughly `spacing` intervals.
pub fn resample_catmull_rom(pts: &[StrokePoint], spacing: f64) -> Vec<StrokePoint> {
    if pts.len() < 3 || spacing <= 0.0 {
        return pts.to_vec();
    }
    let get = |i: isize| pts[i.clamp(0, pts.len() as isize - 1) as usize];
    let mut out = vec![pts[0]];
    for i in 0..pts.len() - 1 {
        let (p0, p1, p2, p3) = (
            get(i as isize - 1),
            get(i as isize),
            get(i as isize + 1),
            get(i as isize + 2),
        );
        let seg_len = p1.pos().dist(p2.pos());
        let steps = (seg_len / spacing).ceil().max(1.0) as usize;
        for s in 1..=steps {
            if s == steps {
                out.push(p2);
                continue;
            }
            let t = s as f64 / steps as f64;
            let pos = catmull(p0.pos(), p1.pos(), p2.pos(), p3.pos(), t);
            let p = p1.p + (p2.p - p1.p) * t;
            out.push(StrokePoint {
                x: pos.x,
                y: pos.y,
                p,
            });
        }
    }
    out
}

fn catmull(p0: V2, p1: V2, p2: V2, p3: V2, t: f64) -> V2 {
    let t2 = t * t;
    let t3 = t2 * t;
    (p1 * 2.0
        + (p2 - p0) * t
        + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * t2
        + (p1 * 3.0 - p0 - p2 * 3.0 + p3) * t3)
        * 0.5
}

/// Radius at a point for a stroke of nominal `width` and pressure `thinning`.
pub fn radius(width: f64, thinning: f64, p: f64) -> f64 {
    let t = thinning.clamp(0.0, 1.0);
    (width * 0.5 * (1.0 + t * (2.0 * p.clamp(0.0, 1.0) - 1.0))).max(width * 0.08)
}

/// Closed outline polygon of a variable-width stroke, with round caps.
pub fn outline(pts: &[StrokePoint], width: f64, thinning: f64) -> Vec<V2> {
    const CAP_SEGMENTS: usize = 10;
    let circle = |c: V2, r: f64| -> Vec<V2> {
        (0..CAP_SEGMENTS * 2)
            .map(|i| {
                let a = i as f64 / (CAP_SEGMENTS * 2) as f64 * std::f64::consts::TAU;
                c + v2(a.cos(), a.sin()) * r
            })
            .collect()
    };
    // Drop consecutive duplicates; they have no direction.
    let mut p: Vec<StrokePoint> = Vec::with_capacity(pts.len());
    for q in pts {
        if p.last()
            .is_none_or(|l: &StrokePoint| l.pos().dist(q.pos()) > 1e-6)
        {
            p.push(*q);
        }
    }
    match p.len() {
        0 => return Vec::new(),
        1 => return circle(p[0].pos(), radius(width, thinning, p[0].p)),
        _ => {}
    }
    let n = p.len();
    let normals: Vec<V2> = (0..n)
        .map(|i| {
            let a = p[i.saturating_sub(1)].pos();
            let b = p[(i + 1).min(n - 1)].pos();
            (b - a).normalized().perp()
        })
        .collect();
    let radii: Vec<f64> = p.iter().map(|q| radius(width, thinning, q.p)).collect();
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for i in 0..n {
        left.push(p[i].pos() + normals[i] * radii[i]);
        right.push(p[i].pos() - normals[i] * radii[i]);
    }
    let mut out = left;
    // End cap: half circle from the left side to the right side around the last point.
    let cap = |c: V2, from: V2, r: f64, out: &mut Vec<V2>| {
        let a0 = from.y.atan2(from.x);
        for s in 1..CAP_SEGMENTS {
            let a = a0 - std::f64::consts::PI * s as f64 / CAP_SEGMENTS as f64;
            out.push(c + v2(a.cos(), a.sin()) * r);
        }
    };
    cap(p[n - 1].pos(), normals[n - 1], radii[n - 1], &mut out);
    out.extend(right.into_iter().rev());
    cap(p[0].pos(), -normals[0], radii[0], &mut out);
    out
}

/// Bounds of a stroke including its thickness.
pub fn bounds(pts: &[StrokePoint], width: f64, thinning: f64) -> Rect {
    let mut r: Option<Rect> = None;
    for q in pts {
        let rad = radius(width, thinning, q.p);
        let b = Rect::new(q.x - rad, q.y - rad, rad * 2.0, rad * 2.0);
        r = Some(r.map_or(b, |a| a.union(&b)));
    }
    r.unwrap_or_default()
}

/// Shortest distance from `q` to the stroke's centerline.
pub fn distance_to_polyline(pts: &[StrokePoint], q: V2) -> f64 {
    if pts.len() == 1 {
        return pts[0].pos().dist(q);
    }
    pts.windows(2)
        .map(|w| segment_distance(w[0].pos(), w[1].pos(), q))
        .fold(f64::INFINITY, f64::min)
}

pub fn segment_distance(a: V2, b: V2, q: V2) -> f64 {
    let ab = b - a;
    let l2 = ab.dot(ab);
    let t = if l2 < 1e-12 {
        0.0
    } else {
        ((q - a).dot(ab) / l2).clamp(0.0, 1.0)
    };
    (a + ab * t).dist(q)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic noise in [-1, 1].
    fn noise(seed: &mut u64) -> f64 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*seed >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0
    }

    fn mean_abs_y(pts: &[StrokePoint]) -> f64 {
        pts.iter().map(|p| p.y.abs()).sum::<f64>() / pts.len() as f64
    }

    #[test]
    fn smoothing_reduces_jitter() {
        let mut seed = 42;
        let raw: Vec<V2> = (0..200)
            .map(|i| v2(i as f64 * 2.0, noise(&mut seed) * 3.0))
            .collect();
        let raw_dev = raw.iter().map(|p| p.y.abs()).sum::<f64>() / raw.len() as f64;
        let mut b = InkBuilder::new(0.6, 0.1);
        for (i, p) in raw.iter().enumerate() {
            b.push(*p, None, i as f64 * 8.0, 1.0);
        }
        let out = b.finish(None, 1.5);
        let dev = mean_abs_y(&out);
        assert!(dev < raw_dev * 0.6, "smoothed {dev} vs raw {raw_dev}");
        // Smoothing more should help more.
        let mut b2 = InkBuilder::new(0.0, 0.1);
        for (i, p) in raw.iter().enumerate() {
            b2.push(*p, None, i as f64 * 8.0, 1.0);
        }
        assert!(mean_abs_y(&b2.finish(None, 1.5)) > dev);
    }

    #[test]
    fn finish_reaches_endpoint_and_resamples_uniformly() {
        let mut b = InkBuilder::new(0.8, 0.1);
        for i in 0..=50 {
            b.push(v2(i as f64 * 4.0, 0.0), Some(0.5), i as f64 * 16.0, 1.0);
        }
        let out = b.finish(Some(v2(200.0, 0.0)), 2.0);
        assert_eq!(out.last().unwrap().pos(), v2(200.0, 0.0));
        for w in out.windows(2) {
            assert!(
                w[0].pos().dist(w[1].pos()) <= 3.0,
                "spacing {}",
                w[0].pos().dist(w[1].pos())
            );
            assert!(w[1].x >= w[0].x - 1e-9, "monotone along a straight line");
        }
    }

    #[test]
    fn real_pressure_is_kept_and_speed_thins() {
        let mut b = InkBuilder::new(0.0, 0.1);
        b.push(v2(0.0, 0.0), Some(0.9), 0.0, 1.0);
        b.push(v2(10.0, 0.0), Some(0.2), 10.0, 1.0);
        assert_eq!(b.points()[1].p, 0.2);

        let run = |step: f64| {
            let mut b = InkBuilder::new(0.0, 0.1);
            for i in 0..40 {
                b.push(v2(i as f64 * step, 0.0), None, i as f64 * 8.0, 1.0);
            }
            b.points().last().unwrap().p
        };
        assert!(run(40.0) < run(1.0));
    }

    #[test]
    fn outline_is_closed_and_wraps_points() {
        let pts: Vec<StrokePoint> = (0..20)
            .map(|i| StrokePoint {
                x: i as f64 * 5.0,
                y: (i as f64 * 0.3).sin() * 10.0,
                p: 0.5,
            })
            .collect();
        let o = outline(&pts, 8.0, 0.5);
        assert!(o.len() >= pts.len() * 2 + 10);
        let b = bounds(&pts, 8.0, 0.5);
        for q in &o {
            assert!(b.inflate(1e-6).contains(*q));
        }
        assert!(
            (outline(&pts[..1], 8.0, 0.5).len()) == 20,
            "single dot is a circle"
        );
        assert!(distance_to_polyline(&pts, v2(50.0, 100.0)) > 80.0);
    }

    #[test]
    fn pressure_modulates_radius() {
        assert!(radius(10.0, 0.6, 1.0) > radius(10.0, 0.6, 0.5));
        assert!(radius(10.0, 0.6, 0.0) < radius(10.0, 0.6, 0.5));
        assert_eq!(radius(10.0, 0.0, 0.0), radius(10.0, 0.0, 1.0));
    }
}
