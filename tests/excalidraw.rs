//! Milestone 4: Excalidraw-core features through the editor's input path.

mod common;

use common::*;
use sketch_rs::editor::shortcuts::Action;
use sketch_rs::editor::{Button, Editor, InputEvent, Key, Modifiers, Tool};
use sketch_rs::geom::{Rect, V2, v2};
use sketch_rs::model::rough::{FillStyle, Rough};
use sketch_rs::model::{Color, Head, Node, NodeId, NodeKind, Route};
use sketch_rs::render::draw;
use sketch_rs::render::frame;
use sketch_rs::ui::UiState;

fn shape(e: &mut Editor, kind: NodeKind, r: Rect, fill: Color) -> NodeId {
    let mut n = Node::new(NodeId(0), kind, r);
    n.style.set_fill(Some(fill));
    n.style.stroke = Some(Color::INK);
    n.style.stroke_width = 2.0;
    e.add_node(n, None)
}

fn click(e: &mut Editor, p: V2, t: f64) {
    e.handle(InputEvent::PointerDown {
        pos: p,
        button: Button::Primary,
        pressure: None,
        time_ms: t,
    });
    e.handle(InputEvent::PointerUp {
        pos: p,
        button: Button::Primary,
        time_ms: t + 20.0,
    });
}

fn conn(e: &Editor, id: NodeId) -> (Vec<V2>, Option<NodeId>, Option<NodeId>, Route) {
    let n = e.doc.get(id).unwrap();
    let NodeKind::Connector {
        start, end, route, ..
    } = &n.kind
    else {
        panic!("not a connector")
    };
    (
        e.connector_points(id).unwrap(),
        start.map(|b| b.node),
        end.map(|b| b.node),
        *route,
    )
}

const BLUE: Color = Color::rgb(0xa5, 0xd8, 0xff);
const PINK: Color = Color::rgb(0xff, 0xc9, 0xc9);

#[test]
fn hand_drawn_toggle_is_deterministic() {
    let mut e = editor(800.0, 600.0);
    let r = shape(
        &mut e,
        NodeKind::Rect,
        Rect::new(100.0, 100.0, 160.0, 100.0),
        BLUE,
    );
    e.selection = vec![r];
    key(
        &mut e,
        Key::Char('h'),
        Modifiers {
            alt: true,
            ..Modifiers::cmd()
        },
    );
    let n = e.doc.get(r).unwrap().clone();
    let rough = n.style.rough.expect("hand-drawn on");
    let wr = e.doc.world_rect(r).unwrap();
    let a = draw::node_draw(&n, wr, 1.0);
    let b = draw::node_draw(&n, wr, 1.0);
    assert_eq!(a, b, "same seed, same wobble");
    // Hachure fill (strokes in the fill colour) plus the doubled rough outline.
    assert!(a.ops.len() >= 2);
    let mut n2 = n.clone();
    n2.style.rough = Some(Rough {
        seed: rough.seed + 1,
        ..rough
    });
    assert_ne!(
        draw::node_draw(&n2, wr, 1.0),
        a,
        "a new seed changes the wobble"
    );
    let mut n3 = n.clone();
    n3.style.rough = Some(Rough {
        fill_style: FillStyle::Solid,
        ..rough
    });
    assert!(matches!(
        draw::node_draw(&n3, wr, 1.0).ops[0],
        draw::DrawOp::Fill { .. }
    ));
    // Undo turns it off again.
    e.undo();
    assert!(e.doc.get(r).unwrap().style.rough.is_none());
    // With nothing selected the toggle sets the default for new shapes.
    e.selection.clear();
    key(
        &mut e,
        Key::Char('h'),
        Modifiers {
            alt: true,
            ..Modifiers::cmd()
        },
    );
    e.run(Action::Tool(Tool::Ellipse));
    ev_drag(&mut e, v2(400.0, 300.0), v2(500.0, 380.0), 0.0, true);
    assert!(e.doc.get(e.selection[0]).unwrap().style.rough.is_some());
}

#[test]
fn arrows_bind_follow_and_unbind() {
    let mut e = editor(1000.0, 700.0);
    let a = shape(
        &mut e,
        NodeKind::Rect,
        Rect::new(100.0, 100.0, 120.0, 80.0),
        BLUE,
    );
    let b = shape(
        &mut e,
        NodeKind::Ellipse,
        Rect::new(500.0, 300.0, 120.0, 80.0),
        PINK,
    );
    e.run(Action::Tool(Tool::Arrow));
    ev_drag(&mut e, v2(160.0, 140.0), v2(560.0, 340.0), 0.0, true);
    let arrow = e.selection[0];
    let (pts, s, en, _) = conn(&e, arrow);
    assert_eq!((s, en), (Some(a), Some(b)), "both ends bound");
    // Ends stop just outside the outlines (gap 5 + stroke/2).
    assert!(!e.doc.world_rect(a).unwrap().contains(pts[0]));
    assert!(e.doc.world_rect(a).unwrap().inflate(10.0).contains(pts[0]));
    assert_eq!(e.history.undo_len(), 3, "drawing an arrow is one undo step");
    // Move the ellipse: the arrow follows.
    e.selection = vec![b];
    ev_drag(&mut e, v2(560.0, 340.0), v2(560.0, 540.0), 1000.0, true);
    let (pts2, _, _, _) = conn(&e, arrow);
    assert!(
        pts2.last().unwrap().y > pts.last().unwrap().y + 150.0,
        "{pts2:?}"
    );
    // The other end stays on A's outline, sliding toward the new direction (orbit binding).
    let ar = e.doc.world_rect(a).unwrap();
    assert!(
        !ar.contains(pts2[0]) && ar.inflate(10.0).contains(pts2[0]),
        "{pts2:?}"
    );
    // Undo the move restores the arrow too.
    e.undo();
    assert_eq!(conn(&e, arrow).0, pts);
    // Elbow routing: orthogonal segments.
    sketch_rs::ui::inspector::apply(&mut e, "route", &[arrow], |n| {
        if let NodeKind::Connector { route, .. } = &mut n.kind {
            *route = Route::Elbow;
        }
    });
    let (ep, _, _, route) = conn(&e, arrow);
    assert_eq!(route, Route::Elbow);
    assert!(ep.len() >= 3);
    for w in ep.windows(2) {
        assert!(
            (w[0].x - w[1].x).abs() < 1e-6 || (w[0].y - w[1].y).abs() < 1e-6,
            "{ep:?}"
        );
    }
    // Deleting a bound shape drops the binding but keeps the arrow.
    e.selection = vec![a];
    key(&mut e, Key::Delete, Modifiers::NONE);
    let (_, s2, _, _) = conn(&e, arrow);
    assert_eq!(s2, None);
    e.doc.validate().unwrap();
}

#[test]
fn arrow_endpoint_drag_rebinds_and_bend_curves() {
    let mut e = editor(1000.0, 700.0);
    let a = shape(
        &mut e,
        NodeKind::Rect,
        Rect::new(100.0, 100.0, 100.0, 100.0),
        BLUE,
    );
    let c = shape(
        &mut e,
        NodeKind::Polygon { sides: 4 },
        Rect::new(400.0, 400.0, 120.0, 100.0),
        PINK,
    );
    e.run(Action::Tool(Tool::Arrow));
    ev_drag(&mut e, v2(150.0, 150.0), v2(700.0, 150.0), 0.0, true);
    let arrow = e.selection[0];
    assert_eq!(conn(&e, arrow).2, None, "free end");
    // Drag the free end onto the diamond.
    let end = e.camera.world_to_screen(*conn(&e, arrow).0.last().unwrap());
    ev_drag(&mut e, end, v2(460.0, 450.0), 1000.0, true);
    assert_eq!(conn(&e, arrow).2, Some(c));
    // Bend from the midpoint handle.
    let mid = e
        .connector_handles()
        .into_iter()
        .find(|(i, _)| *i == 2)
        .unwrap()
        .1;
    ev_drag(&mut e, mid, mid + v2(-80.0, 120.0), 2000.0, true);
    let (pts, _, _, route) = conn(&e, arrow);
    assert_eq!(route, Route::Curved);
    assert_eq!(pts.len(), 3);
    let _ = a;
}

#[test]
fn quick_connect_and_sticky_notes() {
    let mut e = editor(1000.0, 700.0);
    let a = shape(
        &mut e,
        NodeKind::Rect,
        Rect::new(100.0, 200.0, 140.0, 80.0),
        BLUE,
    );
    e.selection = vec![a];
    let knobs = e.quick_knobs();
    assert_eq!(knobs.len(), 4);
    let (_, right) = knobs[0];
    click(&mut e, right, 0.0);
    let b = e.selection[0];
    assert_ne!(a, b);
    assert_eq!(
        e.doc.world_rect(b).unwrap(),
        Rect::new(320.0, 200.0, 140.0, 80.0)
    );
    let arrow = e
        .doc
        .nodes
        .values()
        .find(|n| matches!(n.kind, NodeKind::Connector { .. }))
        .unwrap()
        .id;
    assert_eq!((conn(&e, arrow).1, conn(&e, arrow).2), (Some(a), Some(b)));

    // Sticky: click places a note and starts typing.
    e.run(Action::Tool(Tool::Sticky));
    click(&mut e, v2(700.0, 400.0), 1000.0);
    let s = e.selection[0];
    assert!(matches!(
        e.doc.get(s).unwrap().kind,
        NodeKind::Sticky { .. }
    ));
    assert!(e.text_edit.is_some());
    e.handle(InputEvent::Text("Ship it".into()));
    key(&mut e, Key::Escape, Modifiers::NONE);
    let n = e.doc.get(s).unwrap();
    assert_eq!(n.kind.text().unwrap(), "Ship it");
    // Long text shrinks the font to fit.
    let long = "word ".repeat(80);
    sketch_rs::ui::inspector::apply(&mut e, "text", &[s], |n| {
        *n.kind.text_mut().unwrap() = long.clone()
    });
    let l = sketch_rs::text::layout_node(e.doc.get(s).unwrap()).unwrap();
    assert!(l.glyphs[0].size < 24.0, "font shrank: {}", l.glyphs[0].size);
}

#[test]
fn laser_trail_fades_and_stops_animating() {
    let mut e = editor(800.0, 600.0);
    e.run(Action::Tool(Tool::Laser));
    ev_drag(&mut e, v2(100.0, 100.0), v2(400.0, 300.0), 0.0, true);
    assert!(e.doc.nodes.is_empty(), "the laser draws nothing permanent");
    assert!(e.laser.len() > 5);
    assert!(e.animating(300.0));
    assert!(!e.laser_trail(300.0).is_empty());
    e.tick(1500.0);
    assert!(e.laser.is_empty());
    assert!(!e.animating(1500.0), "idle again once faded");
}

#[test]
fn library_items_insert_with_fresh_ids() {
    let mut e = editor(800.0, 600.0);
    let lib = sketch_rs::library::starter();
    let db = lib.items.iter().find(|i| i.name == "Database").unwrap();
    let ids = e.insert_library_item(db, v2(300.0, 300.0));
    assert_eq!(ids.len(), 2);
    let b = Rect::union_all(
        &ids.iter()
            .map(|i| e.doc.world_rect(*i).unwrap())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(b.center().dist(v2(300.0, 300.0)) < 1.0);
    // An item with an arrow between two shapes keeps the binding inside the copy.
    let a = shape(
        &mut e,
        NodeKind::Rect,
        Rect::new(0.0, 0.0, 50.0, 50.0),
        BLUE,
    );
    let c = shape(
        &mut e,
        NodeKind::Rect,
        Rect::new(150.0, 0.0, 50.0, 50.0),
        PINK,
    );
    e.selection = vec![a];
    e.quick_knobs();
    e.run(Action::Tool(Tool::Arrow));
    ev_drag(&mut e, v2(25.0, 25.0), v2(175.0, 25.0), 0.0, true);
    let arrow = e.selection[0];
    let item = sketch_rs::library::LibItem::from_selection(&e.doc, &[a, c, arrow], "Pair").unwrap();
    let new = e.insert_library_item(&item, v2(500.0, 400.0));
    let new_arrow = *new
        .iter()
        .find(|i| matches!(e.doc.get(**i).unwrap().kind, NodeKind::Connector { .. }))
        .unwrap();
    let (_, s, en, _) = conn(&e, new_arrow);
    assert!(
        s.is_some_and(|x| new.contains(&x)) && en.is_some_and(|x| new.contains(&x)),
        "bindings remapped"
    );
    e.doc.validate().unwrap();
}

/// A whiteboard with every milestone-4 feature.
pub fn whiteboard() -> Editor {
    let mut e = editor(1440.0, 900.0);
    e.doc.name = "Retro board".into();
    e.sketch = Some(Rough::default());
    let mk = |e: &mut Editor, kind: NodeKind, r: Rect, fill: Color, fs: FillStyle, rough: f64| {
        let mut n = Node::new(NodeId(0), kind, r);
        n.style.set_fill(Some(fill));
        n.style.stroke = Some(Color::rgb(0x1e, 0x1e, 0x1e));
        n.style.stroke_width = 2.0;
        n.style.corner_radius = 8.0;
        let id = e.add_node(n, None);
        sketch_rs::ui::inspector::apply(e, "rough", &[id], |n| {
            n.style.rough = Some(Rough {
                seed: 100 + id.0 as u32,
                fill_style: fs,
                roughness: rough,
                ..Rough::default()
            })
        });
        id
    };
    let start = mk(
        &mut e,
        NodeKind::Rect,
        Rect::new(40.0, 160.0, 170.0, 90.0),
        Color::rgb(0xb2, 0xf2, 0xbb),
        FillStyle::Hachure,
        1.0,
    );
    let decide = mk(
        &mut e,
        NodeKind::Polygon { sides: 4 },
        Rect::new(300.0, 140.0, 170.0, 130.0),
        Color::rgb(0xff, 0xec, 0x99),
        FillStyle::CrossHatch,
        1.0,
    );
    let yes = mk(
        &mut e,
        NodeKind::Ellipse,
        Rect::new(580.0, 40.0, 160.0, 100.0),
        Color::rgb(0xa5, 0xd8, 0xff),
        FillStyle::Solid,
        1.0,
    );
    let no = mk(
        &mut e,
        NodeKind::Rect,
        Rect::new(580.0, 300.0, 160.0, 90.0),
        Color::rgb(0xff, 0xc9, 0xc9),
        FillStyle::Zigzag,
        2.0,
    );
    for (id, t) in [
        (start, "Start"),
        (decide, "Ready?"),
        (yes, "Ship"),
        (no, "Fix"),
    ] {
        let r = e.doc.world_rect(id).unwrap();
        let tid = e.add_node(
            sketch_rs::demo::text_node(t, 22.0, v2(0.0, 0.0), Color::INK),
            None,
        );
        let tr = e.doc.get(tid).unwrap().rect;
        sketch_rs::ui::inspector::apply(&mut e, "x", &[tid], |n| {
            n.rect.x = (r.center().x - tr.w * 0.5).round();
            n.rect.y = (r.center().y - tr.h * 0.5).round();
        });
    }
    let arrow = |e: &mut Editor, a: NodeId, b: NodeId, route: Route, head: Head| {
        let pa = e
            .camera
            .world_to_screen(e.doc.world_rect(a).unwrap().center());
        let pb = e
            .camera
            .world_to_screen(e.doc.world_rect(b).unwrap().center());
        e.arrow_route = route;
        e.run(Action::Tool(Tool::Arrow));
        ev_drag(e, pa, pb, 0.0, true);
        let id = e.selection[0];
        sketch_rs::ui::inspector::apply(e, "heads", &[id], |n| {
            if let NodeKind::Connector { end_head, .. } = &mut n.kind {
                *end_head = head;
            }
        });
        id
    };
    arrow(&mut e, start, decide, Route::Straight, Head::Arrow);
    arrow(&mut e, decide, yes, Route::Elbow, Head::Triangle);
    arrow(&mut e, decide, no, Route::Curved, Head::Arrow);
    // Bend the curved arrow a little.
    // Sticky notes.
    e.sketch = None;
    for (i, (t, c)) in [
        ("Went well:\nautosave!", 0usize),
        ("Improve:\nfaster export", 1),
        ("Idea:\nlaser pointer for demos", 3),
    ]
    .iter()
    .enumerate()
    {
        e.sticky_color = sketch_rs::ui::inspector::STICKY_COLORS[*c];
        e.run(Action::Tool(Tool::Sticky));
        let p = e
            .camera
            .world_to_screen(v2(900.0 + i as f64 * 190.0, 120.0 + (i % 2) as f64 * 40.0));
        click(&mut e, p, 5000.0 + i as f64 * 1000.0);
        for (k, line) in t.split('\n').enumerate() {
            if k > 0 {
                key(&mut e, Key::Enter, Modifiers::NONE);
            }
            e.handle(InputEvent::Text(line.into()));
        }
        key(&mut e, Key::Escape, Modifiers::NONE);
    }
    // Library item and a hand-drawn freehand arrow.
    let lib = sketch_rs::library::starter();
    let db = lib.items.iter().find(|i| i.name == "Database").unwrap();
    e.insert_library_item(db, v2(1000.0, 420.0));
    e.selection.clear();
    e
}

#[test]
fn m4_whiteboard_screenshot() {
    let Some(g) = gpu() else { return };
    let mut e = whiteboard();
    e.doc.validate().unwrap();
    let mut st = UiState {
        project_name: "Retro board".into(),
        save_state: "Saved".into(),
        show_library: true,
        ..Default::default()
    };
    st.library.library = sketch_rs::library::starter();
    let img = frame::capture(g, &mut e, &mut st, [1600.0, 1000.0], 2.0, |ed| {
        let b = ed.doc.content_bounds().unwrap();
        ed.camera.fit(&b, ed.viewport, 60.0, 1.0);
        // A laser stroke in progress over the board.
        ed.run(Action::Tool(Tool::Laser));
        let pts: Vec<V2> = (0..40)
            .map(|i| {
                let t = i as f64 / 39.0;
                ed.camera
                    .world_to_screen(v2(250.0 + t * 400.0, 420.0 + (t * 9.0).sin() * 30.0))
            })
            .collect();
        ed.handle(InputEvent::PointerDown {
            pos: pts[0],
            button: Button::Primary,
            pressure: None,
            time_ms: 10_000.0,
        });
        for (i, p) in pts.iter().enumerate() {
            ed.handle(InputEvent::PointerMove {
                pos: *p,
                pressure: None,
                time_ms: 10_000.0 + i as f64 * 8.0,
            });
        }
        ed.tick(10_000.0 + 40.0 * 8.0);
    });
    img.save(shot("milestone4_whiteboard.png")).unwrap();
    // The laser is red on screen.
    assert!(
        count(&img, Rect::new(0.0, 0.0, 1600.0, 1000.0), |p| p[0] > 220
            && p[1] < 80
            && p[2] < 80)
            > 100
    );
}
