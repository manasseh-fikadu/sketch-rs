#![allow(clippy::field_reassign_with_default)]

//! Scenario tests: drive the editor with synthetic input, assert document state.

use super::*;
use crate::geom::Camera;

fn ed() -> Editor {
    let mut e = Editor::default();
    e.viewport = v2(1000.0, 800.0);
    e.camera = Camera {
        pan: V2::ZERO,
        zoom: 1.0,
    };
    e
}

fn down(e: &mut Editor, x: f64, y: f64, t: f64) {
    e.handle(InputEvent::PointerDown {
        pos: v2(x, y),
        button: Button::Primary,
        pressure: None,
        time_ms: t,
    });
}
fn mv(e: &mut Editor, x: f64, y: f64, t: f64) {
    e.handle(InputEvent::PointerMove {
        pos: v2(x, y),
        pressure: None,
        time_ms: t,
    });
}
fn up(e: &mut Editor, x: f64, y: f64, t: f64) {
    e.handle(InputEvent::PointerUp {
        pos: v2(x, y),
        button: Button::Primary,
        time_ms: t,
    });
}
fn drag(e: &mut Editor, from: (f64, f64), to: (f64, f64), t0: f64) {
    down(e, from.0, from.1, t0);
    for i in 1..=10 {
        let k = i as f64 / 10.0;
        mv(
            e,
            from.0 + (to.0 - from.0) * k,
            from.1 + (to.1 - from.1) * k,
            t0 + i as f64 * 16.0,
        );
    }
    up(e, to.0, to.1, t0 + 200.0);
}
fn key(e: &mut Editor, k: Key, m: Modifiers) {
    e.handle(InputEvent::Modifiers(m));
    e.handle(InputEvent::KeyDown {
        key: k,
        repeat: false,
    });
    e.handle(InputEvent::Modifiers(Modifiers::NONE));
}
fn press(e: &mut Editor, c: char) {
    key(e, Key::Char(c), Modifiers::NONE);
}
fn rect_of(e: &Editor, id: NodeId) -> Rect {
    e.doc.world_rect(id).unwrap()
}

// ---------------------------------------------------------------- M1: canvas

#[test]
fn wheel_pans_and_ctrl_wheel_zooms_about_cursor() {
    let mut e = ed();
    e.handle(InputEvent::Scroll {
        delta: v2(30.0, -50.0),
        pos: v2(500.0, 400.0),
    });
    assert_eq!(e.camera.pan, v2(30.0, -50.0));
    let anchor = v2(200.0, 100.0);
    let before = e.screen_to_world(anchor);
    e.handle(InputEvent::Modifiers(Modifiers {
        ctrl: true,
        ..Modifiers::NONE
    }));
    e.handle(InputEvent::Scroll {
        delta: v2(0.0, -100.0),
        pos: anchor,
    });
    assert!(e.camera.zoom > 2.5);
    assert!(e.screen_to_world(anchor).dist(before) < 1e-9);
    e.handle(InputEvent::Zoom {
        factor: 0.5,
        pos: anchor,
    });
    assert!(e.screen_to_world(anchor).dist(before) < 1e-9);
}

#[test]
fn space_drag_and_hand_tool_pan() {
    let mut e = ed();
    e.handle(InputEvent::KeyDown {
        key: Key::Space,
        repeat: false,
    });
    drag(&mut e, (100.0, 100.0), (160.0, 140.0), 0.0);
    e.handle(InputEvent::KeyUp { key: Key::Space });
    assert_eq!(e.camera.pan, v2(-60.0, -40.0));
    assert!(e.doc.nodes.is_empty());
    press(&mut e, 'h');
    drag(&mut e, (100.0, 100.0), (90.0, 100.0), 300.0);
    assert_eq!(e.camera.pan, v2(-50.0, -40.0));
}

#[test]
fn zoom_shortcuts() {
    let mut e = ed();
    key(&mut e, Key::Char('='), Modifiers::cmd());
    assert_eq!(e.camera.zoom, 1.5);
    key(&mut e, Key::Char('-'), Modifiers::cmd());
    key(&mut e, Key::Char('-'), Modifiers::cmd());
    assert_eq!(e.camera.zoom, 0.75);
    key(&mut e, Key::Char('0'), Modifiers::cmd());
    assert_eq!(e.camera.zoom, 1.0);
    e.add_node(
        Node::new(
            NodeId(0),
            NodeKind::Rect,
            Rect::new(5000.0, 5000.0, 100.0, 100.0),
        ),
        None,
    );
    key(&mut e, Key::Char('1'), Modifiers::SHIFT);
    let c = e.camera.world_to_screen(v2(5050.0, 5050.0));
    assert!(c.dist(e.viewport * 0.5) < 1e-6);
}

// ---------------------------------------------------------------- M2: sketching

#[test]
fn pencil_draws_one_smoothed_stroke_per_gesture() {
    let mut e = ed();
    press(&mut e, 'b');
    down(&mut e, 100.0, 100.0, 0.0);
    let mut seed = 7u64;
    for i in 1..=60 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let jitter = ((seed >> 40) % 5) as f64 - 2.0;
        mv(
            &mut e,
            100.0 + i as f64 * 5.0,
            100.0 + jitter,
            i as f64 * 8.0,
        );
    }
    up(&mut e, 400.0, 100.0, 500.0);
    assert_eq!(e.doc.nodes.len(), 1);
    assert_eq!(e.history.undo_len(), 1, "a whole stroke is one undo step");
    let n = e.doc.nodes.values().next().unwrap();
    let NodeKind::Stroke { points, width, .. } = &n.kind else {
        panic!("not a stroke")
    };
    assert!(points.len() > 60, "resampled densely: {}", points.len());
    assert_eq!(*width, e.ink_width);
    let r = rect_of(&e, n.id);
    assert!(r.x < 100.0 && r.right() > 400.0 && r.h < 12.0, "{r:?}");
    // Still the pencil, so the next stroke starts immediately.
    assert_eq!(e.tool, Tool::Pencil);
    e.undo();
    assert!(e.doc.nodes.is_empty());
}

#[test]
fn stroke_width_is_screen_constant_when_zoomed() {
    let mut e = ed();
    e.camera.zoom = 4.0;
    press(&mut e, 'b');
    drag(&mut e, (10.0, 10.0), (200.0, 50.0), 0.0);
    let n = e.doc.nodes.values().next().unwrap();
    let NodeKind::Stroke { width, .. } = n.kind else {
        panic!()
    };
    assert_eq!(width, e.ink_width / 4.0);
}

#[test]
fn eraser_removes_touched_strokes_only() {
    let mut e = ed();
    press(&mut e, 'b');
    drag(&mut e, (100.0, 100.0), (300.0, 100.0), 0.0);
    drag(&mut e, (100.0, 300.0), (300.0, 300.0), 1000.0);
    let rect = e.add_node(
        Node::new(
            NodeId(0),
            NodeKind::Rect,
            Rect::new(150.0, 180.0, 50.0, 50.0),
        ),
        None,
    );
    press(&mut e, 'e');
    drag(&mut e, (200.0, 60.0), (200.0, 200.0), 2000.0);
    assert_eq!(e.doc.nodes.len(), 2);
    assert!(e.doc.get(rect).is_some());
    e.undo();
    assert_eq!(e.doc.nodes.len(), 3);
}

// ---------------------------------------------------------------- M3: shapes, frames, text

#[test]
fn shape_tools_create_by_drag_and_click() {
    let mut e = ed();
    press(&mut e, 'r');
    drag(&mut e, (100.0, 100.0), (250.0, 180.0), 0.0);
    let r = e.selection[0];
    assert_eq!(rect_of(&e, r), Rect::new(100.0, 100.0, 150.0, 80.0));
    assert_eq!(e.tool, Tool::Select, "returns to select after creating");

    press(&mut e, 'o');
    e.handle(InputEvent::Modifiers(Modifiers::SHIFT));
    drag(&mut e, (400.0, 100.0), (500.0, 140.0), 500.0);
    e.handle(InputEvent::Modifiers(Modifiers::NONE));
    let o = e.selection[0];
    assert_eq!(
        rect_of(&e, o).w,
        rect_of(&e, o).h,
        "shift constrains to a circle"
    );

    press(&mut e, 'l');
    down(&mut e, 600.0, 600.0, 900.0);
    up(&mut e, 600.0, 600.0, 950.0);
    let l = e.selection[0];
    assert!(matches!(e.doc.get(l).unwrap().kind, NodeKind::Line { b, .. } if b == v2(100.0, 0.0)));
    assert_eq!(e.history.undo_len(), 3);
}

#[test]
fn frames_adopt_children_and_clip_hit_testing() {
    let mut e = ed();
    press(&mut e, 'r');
    drag(&mut e, (120.0, 120.0), (160.0, 160.0), 0.0);
    let inner = e.selection[0];
    press(&mut e, 'f');
    drag(&mut e, (100.0, 100.0), (400.0, 300.0), 500.0);
    let frame = e.selection[0];
    assert!(e.doc.get(frame).unwrap().is_frame());
    assert_eq!(
        e.doc.get(inner).unwrap().parent,
        Some(frame),
        "frame drawn around a shape adopts it"
    );
    assert_eq!(e.doc.get(inner).unwrap().rect.min(), v2(20.0, 20.0));
    // New shapes drawn inside a frame become its children.
    press(&mut e, 'o');
    drag(&mut e, (300.0, 200.0), (350.0, 250.0), 900.0);
    let child = e.selection[0];
    assert_eq!(e.doc.get(child).unwrap().parent, Some(frame));
    // Moving the frame (by its label) moves its children.
    let label = e.frame_labels()[0].1;
    drag(
        &mut e,
        (label.x + 5.0, label.y + 5.0),
        (label.x + 55.0, label.y + 5.0),
        1500.0,
    );
    assert_eq!(rect_of(&e, inner).x, 170.0);
    e.doc.validate().unwrap();
}

#[test]
fn text_tool_types_edits_and_removes_empty() {
    let mut e = ed();
    press(&mut e, 't');
    down(&mut e, 100.0, 100.0, 0.0);
    up(&mut e, 100.0, 100.0, 10.0);
    let id = e.text_edit.as_ref().unwrap().id;
    e.handle(InputEvent::Text("Hello".into()));
    // Letters typed while editing must not trigger tool shortcuts.
    e.handle(InputEvent::KeyDown {
        key: Key::Char('r'),
        repeat: false,
    });
    e.handle(InputEvent::Text("r".into()));
    key(&mut e, Key::Enter, Modifiers::NONE);
    e.handle(InputEvent::Text("world".into()));
    key(&mut e, Key::Backspace, Modifiers::NONE);
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert_eq!(e.tool, Tool::Select);
    let NodeKind::Text { text, .. } = &e.doc.get(id).unwrap().kind else {
        panic!()
    };
    assert_eq!(text, "Hellor\nworl");
    let r = rect_of(&e, id);
    assert!(
        r.w > 30.0 && (r.h - 2.0 * text::line_height(16.0)).abs() < 1e-6,
        "{r:?}"
    );
    assert_eq!(
        e.history.undo_len(),
        1,
        "creating + typing is one undo step"
    );

    // Double-click to edit again; caret lands where clicked.
    // Click away from the resize handles (which win within 7px).
    let (px, py) = (r.x + 4.0, r.y + text::line_height(16.0) * 0.5);
    down(&mut e, px, py, 1000.0);
    up(&mut e, px, py, 1010.0);
    down(&mut e, px, py, 1100.0);
    assert_eq!(e.text_edit.as_ref().unwrap().caret, 0);
    e.handle(InputEvent::Text(">".into()));
    key(&mut e, Key::Escape, Modifiers::NONE);
    let NodeKind::Text { text, .. } = &e.doc.get(id).unwrap().kind else {
        panic!()
    };
    assert!(text.starts_with(">Hello"));

    // Empty text boxes disappear without leaving history.
    let before = e.history.undo_len();
    press(&mut e, 't');
    down(&mut e, 500.0, 500.0, 2000.0);
    up(&mut e, 500.0, 500.0, 2010.0);
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert_eq!(e.doc.nodes.len(), 1);
    assert_eq!(e.history.undo_len(), before);
}

// ---------------------------------------------------------------- M4: select, move, resize, snap

fn two_rects(e: &mut Editor) -> (NodeId, NodeId) {
    let a = e.add_node(
        Node::new(
            NodeId(0),
            NodeKind::Rect,
            Rect::new(100.0, 100.0, 100.0, 100.0),
        ),
        None,
    );
    let b = e.add_node(
        Node::new(
            NodeId(0),
            NodeKind::Rect,
            Rect::new(400.0, 300.0, 50.0, 50.0),
        ),
        None,
    );
    (a, b)
}

#[test]
fn click_shift_click_and_marquee_selection() {
    let mut e = ed();
    let (a, b) = two_rects(&mut e);
    down(&mut e, 150.0, 150.0, 0.0);
    up(&mut e, 150.0, 150.0, 10.0);
    assert_eq!(e.selection, vec![a]);
    e.handle(InputEvent::Modifiers(Modifiers::SHIFT));
    down(&mut e, 420.0, 320.0, 1000.0);
    up(&mut e, 420.0, 320.0, 1010.0);
    assert_eq!(e.selection, vec![a, b]);
    down(&mut e, 420.0, 320.0, 2000.0);
    up(&mut e, 420.0, 320.0, 2010.0);
    assert_eq!(e.selection, vec![a], "shift-click toggles off");
    e.handle(InputEvent::Modifiers(Modifiers::NONE));
    down(&mut e, 700.0, 700.0, 3000.0);
    up(&mut e, 700.0, 700.0, 3010.0);
    assert!(e.selection.is_empty(), "click on empty canvas clears");
    drag(&mut e, (50.0, 50.0), (420.0, 320.0), 4000.0);
    assert_eq!(e.selection, vec![a, b]);
    assert!(e.marquee.is_none());
    key(&mut e, Key::Char('a'), Modifiers::cmd());
    assert_eq!(e.selection.len(), 2);
}

#[test]
fn drag_moves_multi_selection_as_one_undo_step() {
    let mut e = ed();
    let (a, b) = two_rects(&mut e);
    e.select(vec![a, b]);
    e.snapping = false;
    drag(&mut e, (150.0, 150.0), (180.7, 140.2), 0.0);
    assert!(rect_of(&e, a).min().dist(v2(130.7, 90.2)) < 1e-9);
    assert!(rect_of(&e, b).min().dist(v2(430.7, 290.2)) < 1e-9);
    e.undo();
    assert_eq!(rect_of(&e, a).min(), v2(100.0, 100.0));
    assert_eq!(rect_of(&e, b).min(), v2(400.0, 300.0));
    e.redo();
    assert!(rect_of(&e, b).min().dist(v2(430.7, 290.2)) < 1e-9);
}

#[test]
fn move_snaps_to_edges_and_pixels() {
    let mut e = ed();
    let (a, b) = two_rects(&mut e);
    e.select(vec![b]);
    // b's left edge (400 -> 196.6) lands within 6px of a's right edge (200).
    down(&mut e, 410.0, 310.0, 0.0);
    mv(&mut e, 300.0, 310.0, 16.0);
    mv(&mut e, 206.6, 150.4, 32.0);
    assert!(!e.guides.is_empty(), "guides shown while snapping");
    up(&mut e, 206.6, 150.4, 48.0);
    let r = rect_of(&e, b);
    assert_eq!(r.x, 200.0);
    assert_eq!(r.y, r.y.round(), "pixel snapped");
    assert!(e.guides.is_empty());
    let _ = a;
}

#[test]
fn resize_handles_with_shift_and_multi_scale() {
    let mut e = ed();
    let (a, b) = two_rects(&mut e);
    e.select(vec![a]);
    e.snapping = false;
    // Bottom-right handle of a at (200, 200).
    drag(&mut e, (200.0, 200.0), (300.0, 250.0), 0.0);
    assert_eq!(rect_of(&e, a), Rect::new(100.0, 100.0, 200.0, 150.0));
    // Shift keeps aspect ratio (4:3).
    e.handle(InputEvent::Modifiers(Modifiers::SHIFT));
    drag(&mut e, (300.0, 250.0), (500.0, 260.0), 1000.0);
    e.handle(InputEvent::Modifiers(Modifiers::NONE));
    let r = rect_of(&e, a);
    assert!((r.w / r.h - 200.0 / 150.0).abs() < 1e-9, "{r:?}");
    e.undo();
    e.undo();
    // Multi-selection scales both proportionally from the bounds' top-left handle.
    e.select(vec![a, b]);
    let b0 = e.selection_bounds().unwrap();
    assert_eq!(b0, Rect::new(100.0, 100.0, 350.0, 250.0));
    drag(&mut e, (450.0, 350.0), (800.0, 600.0), 2000.0);
    assert_eq!(
        e.selection_bounds().unwrap(),
        Rect::new(100.0, 100.0, 700.0, 500.0)
    );
    assert_eq!(rect_of(&e, b), Rect::new(700.0, 500.0, 100.0, 100.0));
}

#[test]
fn nudge_delete_duplicate_copy_paste_and_order() {
    let mut e = ed();
    let (a, b) = two_rects(&mut e);
    e.select(vec![a]);
    key(&mut e, Key::Right, Modifiers::NONE);
    key(&mut e, Key::Down, Modifiers::SHIFT);
    assert_eq!(rect_of(&e, a).min(), v2(101.0, 110.0));
    let n = e.history.undo_len();
    key(&mut e, Key::Right, Modifiers::NONE);
    assert_eq!(e.history.undo_len(), n, "consecutive nudges coalesce");

    key(&mut e, Key::Char('d'), Modifiers::cmd());
    let dup = e.selection[0];
    assert_ne!(dup, a);
    assert_eq!(rect_of(&e, dup).min(), v2(118.0, 126.0));
    assert_eq!(e.doc.root, vec![a, b, dup]);
    key(&mut e, Key::Char('['), Modifiers::cmd_shift());
    assert_eq!(e.doc.root, vec![dup, a, b]);
    key(&mut e, Key::Char(']'), Modifiers::cmd());
    assert_eq!(e.doc.root, vec![a, dup, b]);

    key(&mut e, Key::Char('c'), Modifiers::cmd());
    key(&mut e, Key::Char('v'), Modifiers::cmd());
    key(&mut e, Key::Char('v'), Modifiers::cmd());
    assert_eq!(e.doc.nodes.len(), 5);
    key(&mut e, Key::Delete, Modifiers::NONE);
    assert_eq!(e.doc.nodes.len(), 4);
    assert!(e.selection.is_empty());
    e.doc.validate().unwrap();
}

#[test]
fn copy_paste_frame_with_children_and_undo() {
    let mut e = ed();
    let mut f = Node::new(
        NodeId(0),
        NodeKind::Frame { clip: true },
        Rect::new(0.0, 0.0, 200.0, 200.0),
    );
    f.name = "F".into();
    let f = e.add_node(f, None);
    e.add_node(
        Node::new(NodeId(0), NodeKind::Rect, Rect::new(10.0, 10.0, 20.0, 20.0)),
        Some(f),
    );
    e.select(vec![f]);
    key(&mut e, Key::Char('c'), Modifiers::cmd());
    key(&mut e, Key::Char('v'), Modifiers::cmd());
    let copy = e.selection[0];
    assert_eq!(e.doc.get(copy).unwrap().children.len(), 1);
    assert_eq!(e.doc.nodes.len(), 4);
    e.doc.validate().unwrap();
    e.undo();
    assert_eq!(e.doc.nodes.len(), 2);
    e.doc.validate().unwrap();
}

#[test]
fn dragging_into_and_out_of_frames_reparents() {
    let mut e = ed();
    let f = e.add_node(
        Node::new(
            NodeId(0),
            NodeKind::Frame { clip: true },
            Rect::new(300.0, 300.0, 300.0, 300.0),
        ),
        None,
    );
    let r = e.add_node(
        Node::new(NodeId(0), NodeKind::Rect, Rect::new(50.0, 50.0, 40.0, 40.0)),
        None,
    );
    e.snapping = false;
    drag(&mut e, (70.0, 70.0), (420.0, 420.0), 0.0);
    assert_eq!(e.doc.get(r).unwrap().parent, Some(f));
    assert_eq!(rect_of(&e, r).min(), v2(400.0, 400.0));
    drag(&mut e, (420.0, 420.0), (70.0, 70.0), 1000.0);
    assert_eq!(e.doc.get(r).unwrap().parent, None);
    e.undo();
    assert_eq!(e.doc.get(r).unwrap().parent, Some(f));
    e.doc.validate().unwrap();
}

#[test]
fn locked_and_hidden_nodes_are_not_hit() {
    let mut e = ed();
    let (a, _) = two_rects(&mut e);
    e.select(vec![a]);
    key(&mut e, Key::Char('l'), Modifiers::cmd_shift());
    assert!(e.doc.get(a).unwrap().locked);
    down(&mut e, 150.0, 150.0, 0.0);
    up(&mut e, 150.0, 150.0, 10.0);
    assert!(e.selection.is_empty());
    e.edit("unlock", |tx| tx.node_mut(a).unwrap().locked = false);
    e.select(vec![a]);
    key(&mut e, Key::Char('h'), Modifiers::cmd_shift());
    assert!(!e.doc.get(a).unwrap().visible);
    assert_eq!(hit::hit_test(&e.doc, v2(150.0, 150.0), 1.0), None);
}

#[test]
fn escape_cancels_a_drag_in_progress() {
    let mut e = ed();
    let (a, _) = two_rects(&mut e);
    e.select(vec![a]);
    down(&mut e, 150.0, 150.0, 0.0);
    mv(&mut e, 250.0, 250.0, 16.0);
    assert_ne!(rect_of(&e, a).min(), v2(100.0, 100.0));
    key(&mut e, Key::Escape, Modifiers::NONE);
    assert_eq!(rect_of(&e, a).min(), v2(100.0, 100.0));
    up(&mut e, 250.0, 250.0, 32.0);
    assert_eq!(rect_of(&e, a).min(), v2(100.0, 100.0));
}

// ---------------------------------------------------------------- M5: layers + undo/redo

#[test]
fn undo_redo_round_trips_a_whole_session() {
    let mut e = ed();
    let mut snapshots = vec![e.doc.clone()];
    press(&mut e, 'f');
    drag(&mut e, (0.0, 0.0), (500.0, 400.0), 0.0);
    snapshots.push(e.doc.clone());
    press(&mut e, 'r');
    drag(&mut e, (50.0, 50.0), (150.0, 150.0), 1000.0);
    snapshots.push(e.doc.clone());
    press(&mut e, 'b');
    drag(&mut e, (200.0, 200.0), (300.0, 260.0), 2000.0);
    snapshots.push(e.doc.clone());
    // Esc: pencil -> select tool; Esc: rect -> parent frame; Cmd+A then selects top level.
    key(&mut e, Key::Escape, Modifiers::NONE);
    key(&mut e, Key::Escape, Modifiers::NONE);
    key(&mut e, Key::Char('a'), Modifiers::cmd());
    key(&mut e, Key::Right, Modifiers::SHIFT);
    snapshots.push(e.doc.clone());
    key(
        &mut e,
        Key::Char('g'),
        Modifiers {
            alt: true,
            ..Modifiers::cmd()
        },
    );
    snapshots.push(e.doc.clone());
    key(&mut e, Key::Delete, Modifiers::NONE);
    snapshots.push(e.doc.clone());
    assert!(e.doc.nodes.is_empty());

    for s in snapshots.iter().rev().skip(1) {
        key(&mut e, Key::Char('z'), Modifiers::cmd());
        assert_eq!(&e.doc, s);
    }
    for s in snapshots.iter().skip(1) {
        key(&mut e, Key::Char('z'), Modifiers::cmd_shift());
        assert_eq!(&e.doc, s);
        e.doc.validate().unwrap();
    }
    assert!(e.is_dirty());
    e.mark_saved();
    assert!(!e.is_dirty());
    e.undo();
    assert!(e.is_dirty());
}

#[test]
fn wrap_in_frame_groups_siblings() {
    let mut e = ed();
    let (a, b) = two_rects(&mut e);
    e.select(vec![a, b]);
    e.run(shortcuts::Action::WrapInFrame);
    let f = e.selection[0];
    assert_eq!(e.doc.root, vec![f]);
    assert_eq!(e.doc.get(f).unwrap().children, vec![a, b]);
    assert_eq!(rect_of(&e, f), Rect::new(100.0, 100.0, 350.0, 250.0));
    assert_eq!(rect_of(&e, b).min(), v2(400.0, 300.0));
}
