//! Milestones 1–2: local projects, the home screen, autosave and pages.

mod common;

use common::*;
use sketch_rs::demo;
use sketch_rs::editor::Editor;
use sketch_rs::geom::{Rect, v2};
use sketch_rs::model::{Color, Node, NodeId, NodeKind};
use sketch_rs::project::{PageData, Project, Store};
use sketch_rs::render::frame;
use sketch_rs::session::{self, Session};
use sketch_rs::ui::UiState;
use sketch_rs::ui::home::{HomeAction, HomeView};

fn temp_store(tag: &str) -> Store {
    let root = std::env::temp_dir().join(format!("sketch-rs-{tag}-{:016x}", fastrand::u64(..)));
    Store::open(root).unwrap()
}

/// A few projects with different content, as a user would have them.
fn populate(store: &Store) -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    let showcase = demo::showcase();
    let boards: Vec<(&str, sketch_rs::model::Document)> = vec![
        ("Launch plan", showcase.doc.clone()),
        ("Brand moodboard", demo::moodboard().doc),
        ("Onboarding flow", demo::flow().doc),
        ("Empty ideas", sketch_rs::model::Document::default()),
    ];
    for (i, (name, doc)) in boards.into_iter().enumerate() {
        let p = store
            .create_with_pages(
                name,
                vec![PageData {
                    id: sketch_rs::project::new_id(),
                    name: "Page 1".into(),
                    doc,
                    camera: None,
                }],
            )
            .unwrap();
        let mut m = Project::open(&p.dir).unwrap();
        // Stagger modification times so the grid order is deterministic.
        m.manifest.modified = 1_760_000_000_000 - i as u64 * 3_600_000 * (i as u64 + 1);
        m.write_manifest().unwrap();
        dirs.push(p.dir);
    }
    dirs
}

#[test]
fn m1_home_screen_lists_projects_with_thumbnails() {
    let store = temp_store("home");
    let dirs = populate(&store);
    let Some(g) = gpu() else { return };
    for d in &dirs {
        let p = Project::open(d).unwrap();
        let doc = &p.load_pages().unwrap()[0].doc;
        if let Some(png) = session::render_thumbnail(g, doc).unwrap() {
            std::fs::write(p.thumbnail_path(), png).unwrap();
        }
    }
    // A trashed project shows up in the trash view.
    let (old, _) = store.create("Old sketches").unwrap();
    store.trash(&old.dir).unwrap();

    let mut st = UiState {
        home: Some(HomeView::load(&store, 1_760_000_600_000)),
        ..Default::default()
    };
    let hv = st.home.as_ref().unwrap();
    let names: Vec<&str> = hv
        .entries
        .iter()
        .map(|e| e.manifest.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "Launch plan",
            "Brand moodboard",
            "Onboarding flow",
            "Empty ideas"
        ]
    );
    assert_eq!(
        hv.entries.iter().filter(|e| e.thumbnail.is_some()).count(),
        3
    );
    assert_eq!(hv.trash.len(), 1);

    let mut ed = Editor::default();
    let img = frame::capture(g, &mut ed, &mut st, [1280.0, 800.0], 2.0, |_| {});
    img.save(shot("milestone1_home.png")).unwrap();
    // Thumbnails are drawn: the first card has non-background pixels (the demo frames).
    let card = Rect::new(220.0, 70.0, 232.0, 148.0);
    assert!(count(&img, card, |p| p[0] < 200 || p[2] < 200) > 200);

    st.home.as_mut().unwrap().show_trash = true;
    let img = frame::capture(g, &mut ed, &mut st, [1280.0, 800.0], 2.0, |_| {});
    img.save(shot("milestone1_trash.png")).unwrap();
    std::fs::remove_dir_all(&store.root).ok();
}

#[test]
fn m1_autosave_survives_reopen_and_crash_temp_files() {
    let store = temp_store("autosave");
    let mut sess = Session::new(store.clone());
    let mut ed = Editor::default();
    sess.create(&mut ed, "Crashy").unwrap();
    let dir = sess.project.as_ref().unwrap().dir.clone();
    ed.add_node(
        Node::new(NodeId(0), NodeKind::Rect, Rect::new(0.0, 0.0, 50.0, 40.0)),
        None,
    );
    let t = std::time::Instant::now();
    assert!(!sess.autosave_due(&ed, t), "debounced");
    assert!(sess.autosave_due(&ed, t + session::AUTOSAVE_DELAY));
    sess.save(&mut ed).unwrap();
    // Simulate a crash in the middle of the next write: a stray temp file next to the page.
    let page = Project::open(&dir).unwrap().page_path(ed.active_page_id());
    let tmp = page.with_file_name(format!(
        ".{}.123-deadbeef.tmp",
        page.file_name().unwrap().to_string_lossy()
    ));
    std::fs::write(&tmp, "{ truncated").unwrap();
    let mut ed2 = Editor::default();
    Session::new(store.clone()).open(&mut ed2, &dir).unwrap();
    assert_eq!(ed2.doc.nodes.len(), 1);
    assert!(!tmp.exists(), "stale temp files are cleaned on open");
    std::fs::remove_dir_all(&store.root).ok();
}

#[test]
fn m1_home_actions_rename_duplicate_trash() {
    let store = temp_store("actions");
    let dirs = populate(&store);
    let mut hv = HomeView::load(&store, sketch_rs::project::now_ms());
    hv.actions = vec![
        HomeAction::Rename(dirs[0].clone(), "Q4 launch".into()),
        HomeAction::Duplicate(dirs[1].clone()),
        HomeAction::Trash(dirs[3].clone()),
    ];
    for a in std::mem::take(&mut hv.actions) {
        match a {
            HomeAction::Rename(d, n) => {
                store.rename(&d, &n).unwrap();
            }
            HomeAction::Duplicate(d) => {
                store.duplicate(&d).unwrap();
            }
            HomeAction::Trash(d) => {
                store.trash(&d).unwrap();
            }
            _ => unreachable!(),
        }
    }
    hv.refresh(&store, sketch_rs::project::now_ms());
    let mut names: Vec<String> = hv.entries.iter().map(|e| e.manifest.name.clone()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "Brand moodboard",
            "Brand moodboard copy",
            "Onboarding flow",
            "Q4 launch"
        ]
    );
    assert_eq!(hv.trash[0].manifest.name, "Empty ideas");
    std::fs::remove_dir_all(&store.root).ok();
}

#[test]
fn m2_pages_keep_separate_canvases_history_and_cameras() {
    let mut ed = editor(1000.0, 700.0);
    ed.add_node(
        Node::new(NodeId(0), NodeKind::Rect, Rect::new(0.0, 0.0, 100.0, 100.0)),
        None,
    );
    ed.camera.pan = v2(-50.0, -50.0);
    let p2 = ed.add_page();
    assert_eq!(ed.active_page, p2);
    assert!(
        ed.doc.nodes.is_empty(),
        "a new page is a fresh infinite canvas"
    );
    ed.add_node(
        Node::new(
            NodeId(0),
            NodeKind::Ellipse,
            Rect::new(5000.0, -3000.0, 10.0, 10.0),
        ),
        None,
    );
    ed.add_node(
        Node::new(
            NodeId(0),
            NodeKind::Ellipse,
            Rect::new(-90000.0, 7e5, 10.0, 10.0),
        ),
        None,
    );
    ed.switch_page(0);
    assert_eq!(ed.doc.nodes.len(), 1);
    assert_eq!(ed.camera.pan, v2(-50.0, -50.0), "camera is per page");
    ed.undo();
    assert!(ed.doc.nodes.is_empty(), "undo is per page");
    ed.switch_page(1);
    assert_eq!(ed.doc.nodes.len(), 2);
    ed.undo();
    assert_eq!(ed.doc.nodes.len(), 1);
    ed.redo();
    assert_eq!(ed.doc.nodes.len(), 2);
    let b = ed.doc.content_bounds().unwrap();
    assert!(
        b.w > 90000.0 && b.h > 7e5,
        "content spans a huge canvas: {b:?}"
    );

    ed.rename_page(1, "Far away");
    let dup = ed.duplicate_page(1);
    assert_eq!(ed.pages[dup].name, "Far away copy");
    assert_eq!(ed.doc.nodes.len(), 2);
    ed.move_page(dup, 0);
    assert_eq!(ed.pages[0].name, "Far away copy");
    assert_eq!(ed.active_page, 0);
    assert!(ed.delete_page(0).is_some());
    assert_eq!(ed.pages.len(), 2);
    assert_eq!(ed.page_name(), "Page 1");
    assert!(ed.delete_page(0).is_some());
    assert!(
        ed.delete_page(0).is_none(),
        "the last page can't be deleted"
    );
}

#[test]
fn m2_copy_paste_between_pages_and_projects() {
    let mut ed = editor(1000.0, 700.0);
    let mut n = Node::new(NodeId(0), NodeKind::Rect, Rect::new(10.0, 20.0, 30.0, 40.0));
    n.style.set_fill(Some(Color::rgb(200, 10, 10)));
    let id = ed.add_node(n, None);
    ed.selection = vec![id];
    key(
        &mut ed,
        sketch_rs::editor::Key::Char('c'),
        sketch_rs::editor::Modifiers::cmd(),
    );
    ed.add_page();
    key(
        &mut ed,
        sketch_rs::editor::Key::Char('v'),
        sketch_rs::editor::Modifiers::cmd(),
    );
    assert_eq!(ed.doc.nodes.len(), 1);
    let pasted = ed.doc.get(ed.selection[0]).unwrap();
    assert_eq!(pasted.style.fill(), Some(Color::rgb(200, 10, 10)));
    assert_eq!(
        pasted.rect.min(),
        v2(10.0, 20.0),
        "paste on another page keeps the position"
    );

    // Another project: the clipboard survives opening a different project.
    let store = temp_store("clip");
    let mut sess = Session::new(store.clone());
    sess.create(&mut ed, "Other").unwrap();
    assert!(ed.doc.nodes.is_empty());
    key(
        &mut ed,
        sketch_rs::editor::Key::Char('v'),
        sketch_rs::editor::Modifiers::cmd(),
    );
    assert_eq!(ed.doc.nodes.len(), 1);
    std::fs::remove_dir_all(&store.root).ok();
}

#[test]
fn m2_pages_panel_screenshot() {
    let Some(g) = gpu() else { return };
    let page = |name: &str, doc: sketch_rs::model::Document| PageData {
        id: sketch_rs::project::new_id(),
        name: name.into(),
        doc,
        camera: None,
    };
    let mut ed = Editor::default();
    ed.open_pages(
        vec![
            page("Launch plan", demo::showcase().doc),
            page("Moodboard", demo::moodboard().doc),
            page("Flows", demo::flow().doc),
            page("Archive", sketch_rs::model::Document::default()),
        ],
        None,
    );
    ed.switch_page(2);
    assert_eq!(ed.page_name(), "Flows");
    let mut st = UiState {
        project_name: "Q4 launch".into(),
        save_state: "Saved".into(),
        ..Default::default()
    };
    let img = frame::capture(g, &mut ed, &mut st, [1440.0, 900.0], 2.0, |ed| {
        ed.zoom_to_fit()
    });
    img.save(shot("milestone2_pages.png")).unwrap();
    // The canvas shows the Flows page: its blue buttons are on screen.
    let blue = Color::rgb(0x2f, 0x6f, 0xf3);
    assert!(
        count(&img, Rect::new(260.0, 60.0, 900.0, 800.0), |p| close(
            p, blue, 12
        )) > 500
    );
}
