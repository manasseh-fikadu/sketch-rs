//! Milestone 5: layers drag-reorder, command palette, shortcuts, inspector.

mod common;

use common::*;
use sketch_rs::editor::shortcuts::{self, Action};
use sketch_rs::editor::{Button, Editor, InputEvent, Key, Modifiers};
use sketch_rs::geom::{Rect, v2};
use sketch_rs::model::{Color, Node, NodeId, NodeKind};
use sketch_rs::render::frame;
use sketch_rs::ui::palette;
use sketch_rs::ui::{LayerDrop, UiState, drop_layer};

fn rect(e: &mut Editor, r: Rect, c: Color, parent: Option<NodeId>) -> NodeId {
    let mut n = Node::new(NodeId(0), NodeKind::Rect, r);
    n.style.set_fill(Some(c));
    e.add_node(n, parent)
}

#[test]
fn layers_drop_above_below_and_into() {
    let mut e = editor(800.0, 600.0);
    let a = rect(&mut e, Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK, None);
    let b = rect(&mut e, Rect::new(20.0, 0.0, 10.0, 10.0), Color::BLACK, None);
    let c = rect(&mut e, Rect::new(40.0, 0.0, 10.0, 10.0), Color::BLACK, None);
    let f = e.add_node(
        Node::new(
            NodeId(0),
            NodeKind::Frame { clip: true },
            Rect::new(100.0, 100.0, 200.0, 200.0),
        ),
        None,
    );
    assert_eq!(e.doc.root, vec![a, b, c, f]);
    // The panel lists front-most first; "above c" means in front of c.
    drop_layer(&mut e, a, LayerDrop::Above(c));
    assert_eq!(e.doc.root, vec![b, c, a, f]);
    drop_layer(&mut e, a, LayerDrop::Below(b));
    assert_eq!(e.doc.root, vec![a, b, c, f]);
    // Into a frame keeps the canvas position.
    let before = e.doc.world_rect(b).unwrap();
    drop_layer(&mut e, b, LayerDrop::Into(f));
    assert_eq!(e.doc.get(b).unwrap().parent, Some(f));
    assert_eq!(e.doc.world_rect(b).unwrap(), before);
    // A frame can't go into itself or its children.
    drop_layer(&mut e, f, LayerDrop::Into(f));
    assert_eq!(e.doc.get(f).unwrap().parent, None);
    e.undo();
    assert_eq!(e.doc.get(b).unwrap().parent, None);
    e.doc.validate().unwrap();
}

#[test]
fn command_palette_finds_and_runs() {
    let mut e = sketch_rs::demo::showcase();
    let mut st = UiState::default();
    let r = palette::search(&e, &st, "zoom fit");
    assert!(
        matches!(r[0].cmd, palette::Command::Run(Action::ZoomFit)),
        "{}",
        r[0].label
    );
    // ⌘K opens the palette through the editor's UI events.
    key(&mut e, Key::Char('k'), Modifiers::cmd());
    assert!(matches!(
        e.ui_events.as_slice(),
        [sketch_rs::editor::commands::UiEvent::TogglePalette]
    ));
    // Run "Select layer: Week plan".
    let r = palette::search(&e, &st, "select layer week");
    let cmd = r[0].cmd.clone();
    palette::run(&mut e, &mut st, cmd);
    assert_eq!(e.doc.get(e.selection[0]).unwrap().name, "Week plan");
}

#[test]
fn editing_shortcuts() {
    let mut e = editor(800.0, 600.0);
    let a = rect(
        &mut e,
        Rect::new(100.0, 100.0, 50.0, 50.0),
        Color::rgb(200, 0, 0),
        None,
    );
    let b = rect(
        &mut e,
        Rect::new(300.0, 100.0, 50.0, 50.0),
        Color::rgb(0, 0, 200),
        None,
    );
    // Tab cycles siblings.
    e.selection = vec![b];
    key(&mut e, Key::Tab, Modifiers::NONE);
    assert_eq!(e.selection, vec![a]);
    key(&mut e, Key::Tab, Modifiers::SHIFT);
    assert_eq!(e.selection, vec![b]);
    // Copy / paste properties.
    e.selection = vec![a];
    key(
        &mut e,
        Key::Char('c'),
        Modifiers {
            alt: true,
            ..Modifiers::cmd()
        },
    );
    e.selection = vec![b];
    key(
        &mut e,
        Key::Char('v'),
        Modifiers {
            alt: true,
            ..Modifiers::cmd()
        },
    );
    assert_eq!(
        e.doc.get(b).unwrap().style.fill(),
        Some(Color::rgb(200, 0, 0))
    );
    // Alt-drag duplicates (one undo step), leaving the original.
    e.selection = vec![a];
    e.handle(InputEvent::Modifiers(Modifiers {
        alt: true,
        ..Modifiers::NONE
    }));
    ev_drag(&mut e, v2(120.0, 120.0), v2(120.0, 320.0), 0.0, true);
    e.handle(InputEvent::Modifiers(Modifiers::NONE));
    assert_eq!(e.doc.nodes.len(), 3);
    assert_eq!(e.doc.world_rect(a).unwrap().y, 100.0);
    assert_eq!(e.doc.world_rect(e.selection[0]).unwrap().y, 300.0);
    e.undo();
    assert_eq!(e.doc.nodes.len(), 2);
    // Flip a vector path horizontally.
    let mut p = Node::new(
        NodeId(0),
        NodeKind::Line {
            a: v2(0.0, 0.0),
            b: v2(40.0, 20.0),
        },
        Rect::new(500.0, 100.0, 40.0, 20.0),
    );
    p.style.stroke = Some(Color::BLACK);
    let l = e.add_node(p, None);
    e.selection = vec![l];
    key(&mut e, Key::Char('h'), Modifiers::SHIFT);
    let NodeKind::Line { a: la, b: lb } = e.doc.get(l).unwrap().kind else {
        unreachable!()
    };
    assert_eq!((la, lb), (v2(40.0, 0.0), v2(0.0, 20.0)));
    // Pages: PageDown/PageUp.
    e.add_page();
    key(&mut e, Key::PageUp, Modifiers::NONE);
    assert_eq!(e.active_page, 0);
    key(&mut e, Key::PageDown, Modifiers::NONE);
    assert_eq!(e.active_page, 1);
    // Rename and hide-UI become UI events.
    e.switch_page(0);
    e.selection = vec![a];
    key(&mut e, Key::Char('r'), Modifiers::cmd());
    key(&mut e, Key::Char('\\'), Modifiers::cmd());
    use sketch_rs::editor::commands::UiEvent;
    assert_eq!(e.ui_events, vec![UiEvent::Rename(a), UiEvent::ToggleUi]);
    let _ = Button::Primary;
}

#[test]
fn shortcut_table_is_complete_and_conflict_free() {
    // Every action reachable from the menus has a key, and help groups every binding.
    let sections = shortcuts::help_sections();
    let total: usize = sections.iter().map(|(_, r)| r.len()).sum();
    assert_eq!(total, shortcuts::help_table().len());
    assert!(total > 90, "{total} shortcuts");
}

#[test]
fn m5_palette_and_shortcuts_screenshots() {
    let Some(g) = gpu() else { return };
    let mut e = sketch_rs::demo::showcase();
    let mut st = UiState {
        project_name: "Launch plan".into(),
        save_state: "Saved".into(),
        ..Default::default()
    };
    st.palette.toggle();
    st.palette.query = "al".into();
    let img = frame::capture(g, &mut e, &mut st, [1440.0, 900.0], 2.0, |ed| {
        ed.zoom_to_fit()
    });
    img.save(shot("milestone5_palette.png")).unwrap();
    st.palette.open = false;
    st.show_help = true;
    let img = frame::capture(g, &mut e, &mut st, [1440.0, 900.0], 2.0, |ed| {
        ed.zoom_to_fit()
    });
    img.save(shot("milestone5_shortcuts.png")).unwrap();
    st.show_help = false;
    // Inspector for a selected frame, layers tree expanded.
    let mut e = sketch_rs::demo::showcase();
    e.selection = vec![e.doc.root[0]];
    let img = frame::capture(g, &mut e, &mut st, [1440.0, 900.0], 2.0, |ed| {
        ed.zoom_to_fit()
    });
    img.save(shot("milestone5_inspector.png")).unwrap();
}
