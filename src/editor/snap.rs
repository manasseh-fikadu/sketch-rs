//! Alignment snapping between bounding boxes.

use crate::geom::Rect;

/// Snap distance in screen pixels.
pub const SNAP_PX: f64 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Guide {
    /// Vertical line at world x spanning y0..y1.
    V { x: f64, y0: f64, y1: f64 },
    /// Horizontal line at world y spanning x0..x1.
    H { y: f64, x0: f64, x1: f64 },
    /// A measured distance between two world points, drawn with its length.
    Gap {
        a: crate::geom::V2,
        b: crate::geom::V2,
    },
}

fn anchors_x(r: &Rect) -> [f64; 3] {
    [r.x, r.x + r.w * 0.5, r.right()]
}
fn anchors_y(r: &Rect) -> [f64; 3] {
    [r.y, r.y + r.h * 0.5, r.bottom()]
}

/// Best correction that brings any of `moving` onto any of `targets` within `threshold`.
fn best_offset(
    moving: &[f64],
    targets: impl Iterator<Item = f64> + Clone,
    threshold: f64,
) -> Option<f64> {
    let mut best: Option<f64> = None;
    for m in moving {
        for t in targets.clone() {
            let d = t - m;
            if d.abs() <= threshold && best.is_none_or(|b| d.abs() < b.abs()) {
                best = Some(d);
            }
        }
    }
    best
}

/// Snap a moving rectangle. `use_x/use_y` select which anchors of `moving` may snap per axis
/// (left, center, right / top, middle, bottom). Returns the correction and the guides.
pub fn snap_rect(
    moving: &Rect,
    use_x: [bool; 3],
    use_y: [bool; 3],
    candidates: &[Rect],
    threshold: f64,
) -> (f64, f64, Vec<Guide>) {
    let mx: Vec<f64> = anchors_x(moving)
        .into_iter()
        .zip(use_x)
        .filter(|(_, u)| *u)
        .map(|(v, _)| v)
        .collect();
    let my: Vec<f64> = anchors_y(moving)
        .into_iter()
        .zip(use_y)
        .filter(|(_, u)| *u)
        .map(|(v, _)| v)
        .collect();
    let dx = best_offset(&mx, candidates.iter().flat_map(anchors_x), threshold).unwrap_or(0.0);
    let dy = best_offset(&my, candidates.iter().flat_map(anchors_y), threshold).unwrap_or(0.0);
    let snapped = Rect::new(moving.x + dx, moving.y + dy, moving.w, moving.h);
    let mut guides = Vec::new();
    let eps = 1e-6;
    if dx != 0.0
        || mx.iter().any(|m| {
            candidates
                .iter()
                .flat_map(anchors_x)
                .any(|t| (t - m).abs() < eps)
        })
    {
        for (v, used) in anchors_x(&snapped).into_iter().zip(use_x) {
            if !used {
                continue;
            }
            for c in candidates
                .iter()
                .filter(|c| anchors_x(c).iter().any(|t| (t - v).abs() < eps))
            {
                let u = snapped.union(c);
                guides.push(Guide::V {
                    x: v,
                    y0: u.y,
                    y1: u.bottom(),
                });
            }
        }
    }
    if dy != 0.0
        || my.iter().any(|m| {
            candidates
                .iter()
                .flat_map(anchors_y)
                .any(|t| (t - m).abs() < eps)
        })
    {
        for (v, used) in anchors_y(&snapped).into_iter().zip(use_y) {
            if !used {
                continue;
            }
            for c in candidates
                .iter()
                .filter(|c| anchors_y(c).iter().any(|t| (t - v).abs() < eps))
            {
                let u = snapped.union(c);
                guides.push(Guide::H {
                    y: v,
                    x0: u.x,
                    x1: u.right(),
                });
            }
        }
    }
    (dx, dy, guides)
}

/// Equal-spacing snap: when the moving rect sits between two neighbors on an axis, make the
/// two gaps equal. Returns the correction and gap guides (drawn with their distances).
pub fn snap_spacing(moving: &Rect, candidates: &[Rect], threshold: f64) -> (f64, f64, Vec<Guide>) {
    use crate::geom::v2;
    let mut guides = Vec::new();
    let mut out = [0.0, 0.0];
    #[allow(clippy::needless_range_loop)]
    for axis in 0..2 {
        let clo = |r: &Rect| {
            if axis == 0 {
                (r.y, r.bottom())
            } else {
                (r.x, r.right())
            }
        };
        let (mlo, mhi) = lo_hi(moving, axis);
        let (mclo, mchi) = (clo)(moving);
        let overlaps = |r: &Rect| {
            let (a, b) = (clo)(r);
            a < mchi && b > mclo
        };
        let before = candidates
            .iter()
            .filter(|r| overlaps(r) && lo_hi(r, axis).1 <= mlo + threshold)
            .max_by(|a, b| lo_hi(a, axis).1.total_cmp(&lo_hi(b, axis).1));
        let after = candidates
            .iter()
            .filter(|r| overlaps(r) && lo_hi(r, axis).0 >= mhi - threshold)
            .min_by(|a, b| lo_hi(a, axis).0.total_cmp(&lo_hi(b, axis).0));
        let (Some(b), Some(a)) = (before, after) else {
            continue;
        };
        let g1 = mlo - lo_hi(b, axis).1;
        let g2 = lo_hi(a, axis).0 - mhi;
        let d = (g2 - g1) * 0.5;
        if d.abs() > threshold || g1 + d < 0.0 {
            continue;
        }
        out[axis] = d;
        let gap = g1 + d;
        // Guides along the middle of the overlap.
        let mid = |r: &Rect| {
            let (a0, a1) = (clo)(r);
            (a0.max(mclo) + a1.min(mchi)) * 0.5
        };
        let (m1, m2) = (mid(b), mid(a));
        let blo = lo_hi(b, axis).1;
        let ahi = lo_hi(a, axis).0;
        if axis == 0 {
            guides.push(Guide::Gap {
                a: v2(blo, m1),
                b: v2(blo + gap, m1),
            });
            guides.push(Guide::Gap {
                a: v2(ahi - gap, m2),
                b: v2(ahi, m2),
            });
        } else {
            guides.push(Guide::Gap {
                a: v2(m1, blo),
                b: v2(m1, blo + gap),
            });
            guides.push(Guide::Gap {
                a: v2(m2, ahi - gap),
                b: v2(m2, ahi),
            });
        }
    }
    (out[0], out[1], guides)
}

fn lo_hi(r: &Rect, axis: usize) -> (f64, f64) {
    if axis == 0 {
        (r.x, r.right())
    } else {
        (r.y, r.bottom())
    }
}

/// Red measurement lines from `sel` to `other` (Alt-hover), as Gap guides.
pub fn measure(sel: &Rect, other: &Rect) -> Vec<Guide> {
    use crate::geom::v2;
    let mut out = Vec::new();
    if other.contains_rect(sel) {
        // Inside a container: distances to each edge.
        let cy = sel.center().y;
        let cx = sel.center().x;
        for (a, b) in [
            (v2(other.x, cy), v2(sel.x, cy)),
            (v2(sel.right(), cy), v2(other.right(), cy)),
            (v2(cx, other.y), v2(cx, sel.y)),
            (v2(cx, sel.bottom()), v2(cx, other.bottom())),
        ] {
            if a.dist(b) > 0.01 {
                out.push(Guide::Gap { a, b });
            }
        }
        return out;
    }
    let ov_y = (sel.y.max(other.y), sel.bottom().min(other.bottom()));
    let ov_x = (sel.x.max(other.x), sel.right().min(other.right()));
    let y = if ov_y.0 < ov_y.1 {
        (ov_y.0 + ov_y.1) * 0.5
    } else {
        sel.center().y
    };
    let x = if ov_x.0 < ov_x.1 {
        (ov_x.0 + ov_x.1) * 0.5
    } else {
        sel.center().x
    };
    if other.x >= sel.right() {
        out.push(Guide::Gap {
            a: v2(sel.right(), y),
            b: v2(other.x, y),
        });
    } else if other.right() <= sel.x {
        out.push(Guide::Gap {
            a: v2(other.right(), y),
            b: v2(sel.x, y),
        });
    }
    if other.y >= sel.bottom() {
        out.push(Guide::Gap {
            a: v2(x, sel.bottom()),
            b: v2(x, other.y),
        });
    } else if other.bottom() <= sel.y {
        out.push(Guide::Gap {
            a: v2(x, other.bottom()),
            b: v2(x, sel.y),
        });
    }
    out
}

#[cfg(test)]
mod spacing_tests {
    use super::*;

    #[test]
    fn spacing_equalizes_gaps() {
        let c = [
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(100.0, 0.0, 10.0, 10.0),
        ];
        let m = Rect::new(42.0, 0.0, 20.0, 10.0); // gaps 32 and 38
        let (dx, dy, g) = snap_spacing(&m, &c, 6.0);
        assert_eq!((dx, dy), (3.0, 0.0));
        assert_eq!(g.len(), 2);
        let m = Rect::new(20.0, 0.0, 20.0, 10.0); // far off
        assert_eq!(snap_spacing(&m, &c, 6.0).0, 0.0);
    }

    #[test]
    fn measures_inside_and_beside() {
        let parent = Rect::new(0.0, 0.0, 100.0, 100.0);
        let sel = Rect::new(10.0, 20.0, 30.0, 30.0);
        assert_eq!(measure(&sel, &parent).len(), 4);
        let beside = Rect::new(60.0, 25.0, 10.0, 10.0);
        let g = measure(&sel, &beside);
        assert!(matches!(g[0], Guide::Gap { a, b } if a.x == 40.0 && b.x == 60.0));
    }

    #[test]
    fn snaps_left_edge_and_centers() {
        let others = [Rect::new(100.0, 0.0, 50.0, 50.0)];
        let m = Rect::new(103.0, 200.0, 10.0, 10.0);
        let (dx, dy, g) = snap_rect(&m, [true; 3], [true; 3], &others, 6.0);
        assert_eq!((dx, dy), (-3.0, 0.0));
        assert!(
            matches!(g[0], Guide::V { x, y0, y1 } if x == 100.0 && y0 == 0.0 && y1 == 210.0),
            "{g:?}"
        );

        let m = Rect::new(114.0, 200.0, 20.0, 20.0); // center 124 -> 125
        let (dx, _, _) = snap_rect(&m, [true; 3], [true; 3], &others, 6.0);
        assert_eq!(dx, 1.0);
    }

    #[test]
    fn respects_threshold_and_axis_mask() {
        let others = [Rect::new(100.0, 100.0, 50.0, 50.0)];
        let m = Rect::new(110.0, 0.0, 10.0, 10.0);
        let (dx, dy, g) = snap_rect(&m, [true; 3], [true; 3], &others, 2.0);
        assert_eq!((dx, dy), (0.0, 0.0));
        assert!(g.is_empty());
        // Only the right edge may snap (resizing from the right).
        let m = Rect::new(96.0, 0.0, 52.0, 10.0); // left 96 (near 100), right 148 (near 150)
        let (dx, _, _) = snap_rect(&m, [false, false, true], [false; 3], &others, 6.0);
        assert_eq!(dx, 2.0);
    }
}
