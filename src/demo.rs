//! Sample content used by `--demo`, screenshots and tests.
//! Sketches are drawn through the real pencil input path (synthetic pointer events).

use crate::editor::{Button, Editor, InputEvent, Tool};
use crate::geom::{Rect, V2, v2};
use crate::model::{Color, Node, NodeId, NodeKind, TextAlign};

pub fn text_node(s: &str, size: f64, at: V2, color: Color) -> Node {
    let mut n = Node::new(
        NodeId(0),
        NodeKind::Text {
            text: s.into(),
            font_size: size,
            auto_width: true,
            align: TextAlign::Left,
        },
        Rect::new(at.x, at.y, 1.0, 1.0),
    );
    n.name = s
        .lines()
        .next()
        .unwrap_or("Text")
        .chars()
        .take(24)
        .collect();
    n.style.set_fill(Some(color));
    n
}

pub fn shape(kind: NodeKind, r: Rect, fill: Option<Color>, radius: f64) -> Node {
    let mut n = Node::new(NodeId(0), kind, r);
    n.style.set_fill(fill);
    n.style.corner_radius = radius;
    n
}

/// Draw a freehand stroke through world points via synthetic pointer events.
pub fn sketch(ed: &mut Editor, world: &[V2], width_px: f64, color: Color) {
    let prev = (ed.tool, ed.ink_width, ed.ink_color);
    ed.tool = Tool::Pencil;
    ed.ink_width = width_px;
    ed.ink_color = color;
    let to_screen = |ed: &Editor, p: V2| ed.camera.world_to_screen(p);
    let mut t = 0.0;
    let s0 = to_screen(ed, world[0]);
    ed.handle(InputEvent::PointerDown {
        pos: s0,
        button: Button::Primary,
        pressure: None,
        time_ms: t,
    });
    for w in world.windows(2) {
        // Interpolate like a real 120 Hz pointer stream.
        let (a, b) = (to_screen(ed, w[0]), to_screen(ed, w[1]));
        let steps = (a.dist(b) / 4.0).ceil().max(1.0) as usize;
        for i in 1..=steps {
            t += 8.0;
            let p = a.lerp(b, i as f64 / steps as f64);
            ed.handle(InputEvent::PointerMove {
                pos: p,
                pressure: None,
                time_ms: t,
            });
        }
    }
    let end = to_screen(ed, *world.last().unwrap());
    ed.handle(InputEvent::PointerUp {
        pos: end,
        button: Button::Primary,
        time_ms: t + 8.0,
    });
    (ed.tool, ed.ink_width, ed.ink_color) = prev;
}

fn wave(origin: V2, len: f64, amp: f64, cycles: f64, n: usize) -> Vec<V2> {
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            origin + v2(t * len, (t * cycles * std::f64::consts::TAU).sin() * amp)
        })
        .collect()
}

fn circle_pts(c: V2, r: f64, n: usize, turns: f64) -> Vec<V2> {
    (0..=n)
        .map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU * turns - 1.2;
            c + v2(
                a.cos() * r * (1.0 + 0.04 * (a * 3.0).sin()),
                a.sin() * r * 0.92,
            )
        })
        .collect()
}

/// A planning board: frames with notes, shapes, text and freehand sketches.
pub fn showcase() -> Editor {
    let mut ed = Editor::default();
    ed.doc.name = "Launch plan".into();
    ed.viewport = v2(1280.0, 800.0);
    let ink = Color::INK;
    let blue = Color::rgb(0x2f, 0x6f, 0xf3);
    let yellow = Color::rgb(0xff, 0xe0, 0x82);
    let pink = Color::rgb(0xff, 0xc2, 0xd1);
    let mint = Color::rgb(0xb8, 0xf0, 0xd2);

    // Frame 1: a planning board.
    let mut f1 = shape(
        NodeKind::Frame { clip: true },
        Rect::new(0.0, 0.0, 640.0, 440.0),
        Some(Color::WHITE),
        0.0,
    );
    f1.name = "Week plan".into();
    let f1 = ed.add_node(f1, None);
    ed.add_node(
        text_node("Week 42 · Launch", 28.0, v2(32.0, 28.0), ink),
        Some(f1),
    );
    ed.add_node(
        text_node(
            "Ship the canvas, then polish.",
            15.0,
            v2(32.0, 72.0),
            Color::rgb(0x6b, 0x6b, 0x75),
        ),
        Some(f1),
    );
    let notes = [
        ("Design review\nMon 10:00", yellow, 32.0),
        ("Fix snapping\nedge cases", pink, 232.0),
        ("Export PNG\n+ SVG", mint, 432.0),
    ];
    for (t, c, x) in notes {
        let mut n = shape(
            NodeKind::Rect,
            Rect::new(x, 120.0, 176.0, 120.0),
            Some(c),
            10.0,
        );
        n.name = "Note".into();
        ed.add_node(n, Some(f1));
        ed.add_node(text_node(t, 16.0, v2(x + 16.0, 136.0), ink), Some(f1));
    }
    let mut bar = shape(
        NodeKind::Rect,
        Rect::new(32.0, 280.0, 576.0, 12.0),
        Some(Color::rgb(0xe7, 0xe7, 0xec)),
        6.0,
    );
    bar.name = "Progress track".into();
    ed.add_node(bar, Some(f1));
    let mut prog = shape(
        NodeKind::Rect,
        Rect::new(32.0, 280.0, 368.0, 12.0),
        Some(blue),
        6.0,
    );
    prog.name = "Progress".into();
    ed.add_node(prog, Some(f1));
    let mut dot = shape(
        NodeKind::Ellipse,
        Rect::new(392.0, 274.0, 24.0, 24.0),
        Some(Color::WHITE),
        0.0,
    );
    dot.style.stroke = Some(blue);
    dot.style.stroke_width = 3.0;
    ed.add_node(dot, Some(f1));
    let mut line = Node::new(
        NodeId(0),
        NodeKind::Line {
            a: v2(0.0, 0.0),
            b: v2(576.0, 0.0),
        },
        Rect::new(32.0, 330.0, 576.0, 0.0),
    );
    line.style.stroke = Some(Color::rgb(0xd4, 0xd4, 0xda));
    line.style.stroke_width = 1.0;
    ed.add_node(line, Some(f1));
    ed.add_node(
        text_node(
            "62% complete — 3 tasks left",
            14.0,
            v2(32.0, 350.0),
            Color::rgb(0x6b, 0x6b, 0x75),
        ),
        Some(f1),
    );

    // Frame 2: a sketch page.
    let mut f2 = shape(
        NodeKind::Frame { clip: true },
        Rect::new(700.0, 0.0, 440.0, 440.0),
        Some(Color::rgb(0xfb, 0xfa, 0xf7)),
        0.0,
    );
    f2.name = "Sketches".into();
    let f2 = ed.add_node(f2, None);
    ed.add_node(text_node("Ideas", 22.0, v2(24.0, 20.0), ink), Some(f2));
    ed.camera = crate::geom::Camera {
        pan: v2(-60.0, -80.0),
        zoom: 1.0,
    };
    sketch(
        &mut ed,
        &circle_pts(v2(840.0, 170.0), 70.0, 90, 1.05),
        3.0,
        ink,
    );
    sketch(
        &mut ed,
        &wave(v2(950.0, 160.0), 150.0, 18.0, 1.5, 60),
        4.0,
        blue,
    );
    sketch(
        &mut ed,
        &wave(v2(760.0, 330.0), 340.0, 10.0, 3.0, 120),
        2.5,
        Color::rgb(0xe0, 0x4f, 0x5f),
    );
    // An arrow.
    sketch(
        &mut ed,
        &[v2(960.0, 260.0), v2(1000.0, 300.0), v2(1060.0, 320.0)],
        3.0,
        ink,
    );
    sketch(
        &mut ed,
        &[v2(1040.0, 300.0), v2(1062.0, 320.0), v2(1036.0, 334.0)],
        3.0,
        ink,
    );

    // Loose canvas content outside frames.
    ed.add_node(
        text_node(
            "Loose notes live on the infinite canvas",
            18.0,
            v2(0.0, 480.0),
            Color::rgb(0x55, 0x55, 0x60),
        ),
        None,
    );
    let mut s = shape(
        NodeKind::Ellipse,
        Rect::new(420.0, 470.0, 46.0, 46.0),
        Some(pink),
        0.0,
    );
    s.name = "Bubble".into();
    ed.add_node(s, None);

    ed.selection.clear();
    ed.tool = Tool::Select;
    ed.mark_saved();
    ed.zoom_to_fit();
    ed.pending_fit = true;
    ed
}

/// A moodboard: color swatches, type specimens and a sketch.
pub fn moodboard() -> Editor {
    let mut ed = Editor::default();
    ed.doc.name = "Brand moodboard".into();
    let ink = Color::INK;
    let mut f = shape(
        NodeKind::Frame { clip: true },
        Rect::new(0.0, 0.0, 720.0, 460.0),
        Some(Color::rgb(0xfb, 0xf7, 0xf0)),
        0.0,
    );
    f.name = "Moodboard".into();
    let f = ed.add_node(f, None);
    let swatches = [
        Color::rgb(0xff, 0x6b, 0x4a),
        Color::rgb(0xff, 0xc8, 0x3d),
        Color::rgb(0x2f, 0x6f, 0xf3),
        Color::rgb(0x1c, 0x1c, 0x1e),
    ];
    for (i, c) in swatches.into_iter().enumerate() {
        let mut s = shape(
            NodeKind::Ellipse,
            Rect::new(40.0 + i as f64 * 110.0, 40.0, 90.0, 90.0),
            Some(c),
            0.0,
        );
        s.name = "Swatch".into();
        ed.add_node(s, Some(f));
    }
    ed.add_node(text_node("Aa", 96.0, v2(40.0, 170.0), ink), Some(f));
    ed.add_node(
        text_node(
            "Warm, bold, friendly.\nInter for everything.",
            20.0,
            v2(200.0, 200.0),
            Color::rgb(0x55, 0x55, 0x5c),
        ),
        Some(f),
    );
    let mut card = shape(
        NodeKind::Rect,
        Rect::new(480.0, 160.0, 200.0, 260.0),
        Some(Color::rgb(0xff, 0x6b, 0x4a)),
        18.0,
    );
    card.name = "Card".into();
    ed.add_node(card, Some(f));
    sketch(
        &mut ed,
        &wave(v2(60.0, 380.0), 360.0, 18.0, 2.5, 60),
        4.0,
        Color::rgb(0x2f, 0x6f, 0xf3),
    );
    ed
}

/// A three-screen onboarding flow with connectors.
pub fn flow() -> Editor {
    let mut ed = Editor::default();
    ed.doc.name = "Onboarding flow".into();
    let blue = Color::rgb(0x2f, 0x6f, 0xf3);
    for (i, title) in ["Welcome", "Pick a plan", "All set"]
        .into_iter()
        .enumerate()
    {
        let x = i as f64 * 300.0;
        let mut f = shape(
            NodeKind::Frame { clip: true },
            Rect::new(x, 0.0, 220.0, 420.0),
            Some(Color::WHITE),
            16.0,
        );
        f.name = title.into();
        let f = ed.add_node(f, None);
        ed.add_node(text_node(title, 22.0, v2(20.0, 40.0), Color::INK), Some(f));
        for k in 0..3 {
            let mut r = shape(
                NodeKind::Rect,
                Rect::new(20.0, 100.0 + k as f64 * 70.0, 180.0, 52.0),
                Some(Color::rgb(0xf0, 0xf0, 0xf4)),
                10.0,
            );
            r.name = "Row".into();
            ed.add_node(r, Some(f));
        }
        let mut b = shape(
            NodeKind::Rect,
            Rect::new(20.0, 350.0, 180.0, 44.0),
            Some(blue),
            22.0,
        );
        b.name = "Button".into();
        ed.add_node(b, Some(f));
        if i < 2 {
            let mut l = Node::new(
                NodeId(0),
                NodeKind::Line {
                    a: v2(0.0, 0.0),
                    b: v2(80.0, 0.0),
                },
                Rect::new(x + 220.0, 210.0, 80.0, 0.0),
            );
            l.style.stroke = Some(blue);
            l.style.stroke_width = 3.0;
            ed.add_node(l, None);
        }
    }
    ed
}
