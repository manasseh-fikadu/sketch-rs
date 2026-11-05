//! Prototype preview: links across pages, scrolling, hover states, overlays and transitions,
//! driven by synthetic input like the rest of the editor, plus GPU pixels for what only shows up
//! when drawn.

mod common;

use common::*;
use serde_json::{Value, json};
use sketch_rs::editor::shortcuts::Action;
use sketch_rs::editor::{AppRequest, Button, Editor, InputEvent, Key, Modifiers};
use sketch_rs::geom::{Rect, V2, v2};
use sketch_rs::mcp::{Out, exec};
use sketch_rs::model::{
    Color, Interaction, Node, NodeId, NodeKind, Overflow, OverlayPlacement, ProtoAction,
    ScrollPosition, SlideDir, StateStyle, Target, Transition, TransitionKind, Trigger,
};
use sketch_rs::preview::{Hit, NodeRef, render};
use sketch_rs::project::Store;
use sketch_rs::render::frame;
use sketch_rs::render::gpu::{CanvasRenderer, Headless, OFFSCREEN_FORMAT};
use sketch_rs::ui::UiState;

const VIEW_W: f64 = 1000.0;
const VIEW_H: f64 = 600.0;

const WHITE: Color = Color::rgb(255, 255, 255);
const RED: Color = Color::rgb(220, 30, 30);
const BLUE: Color = Color::rgb(30, 60, 220);
const GREEN: Color = Color::rgb(30, 180, 70);
const YELLOW: Color = Color::rgb(240, 200, 30);

struct Site {
    e: Editor,
    /// Page "Home", 1000×2000, not at the world origin.
    home: NodeId,
    /// In `home` at (100, 100).
    button: NodeId,
    /// Page "About", 1000×3000.
    about_page: String,
    about: NodeId,
    /// In `about`, 1500 down.
    section: NodeId,
}

fn frame_node(r: Rect, fill: Color) -> Node {
    let mut n = Node::new(NodeId(0), NodeKind::Frame { clip: true }, r);
    n.style.set_fill(Some(fill));
    n
}

fn box_node(r: Rect, fill: Color) -> Node {
    let mut n = Node::new(NodeId(0), NodeKind::Rect, r);
    n.style.set_fill(Some(fill));
    n
}

fn set_interactions(e: &mut Editor, id: NodeId, list: Vec<Interaction>) {
    e.edit("link", |tx| {
        tx.node_mut(id)
            .unwrap()
            .edit_proto(|p| p.interactions = list);
    });
}

fn click_to(target: Target) -> Interaction {
    Interaction {
        trigger: Trigger::Click,
        action: ProtoAction::Navigate {
            target,
            transition: Transition::default(),
        },
    }
}

fn site() -> Site {
    let mut e = editor(VIEW_W, VIEW_H);
    e.rename_page(0, "Home");
    let i = e.add_page_in_background(Some("About"));
    let about_page = e.pages[i].id.clone();
    // `about` has a layer at (20, 20) that goes back.
    let (about, section) = e.with_page(i, |e| {
        let about = e.add_node(frame_node(Rect::new(0.0, 0.0, 1000.0, 3000.0), BLUE), None);
        let section = e.add_node(
            box_node(Rect::new(0.0, 1500.0, 1000.0, 400.0), GREEN),
            Some(about),
        );
        let back = e.add_node(
            box_node(Rect::new(20.0, 20.0, 100.0, 40.0), WHITE),
            Some(about),
        );
        set_interactions(
            e,
            back,
            vec![Interaction {
                trigger: Trigger::Click,
                action: ProtoAction::Back,
            }],
        );
        (about, section)
    });
    let home = e.add_node(
        frame_node(Rect::new(300.0, 50.0, 1000.0, 2000.0), WHITE),
        None,
    );
    let button = e.add_node(
        box_node(Rect::new(100.0, 100.0, 200.0, 50.0), RED),
        Some(home),
    );
    set_interactions(
        &mut e,
        button,
        vec![click_to(Target {
            page: Some(about_page.clone()),
            node: about,
        })],
    );
    Site {
        e,
        home,
        button,
        about_page,
        about,
        section,
    }
}

fn start(s: &mut Site) {
    let home = s.home;
    assert!(s.e.start_preview(Some((0, home))));
}

fn down(e: &mut Editor, p: V2, t: f64) {
    e.handle(InputEvent::PointerDown {
        pos: p,
        button: Button::Primary,
        pressure: None,
        time_ms: t,
    });
}

fn up(e: &mut Editor, p: V2, t: f64) {
    e.handle(InputEvent::PointerUp {
        pos: p,
        button: Button::Primary,
        time_ms: t,
    });
}

fn click(e: &mut Editor, p: V2, t: f64) {
    hover(e, p, t);
    down(e, p, t);
    up(e, p, t);
}

fn hover(e: &mut Editor, p: V2, t: f64) {
    e.handle(InputEvent::PointerMove {
        pos: p,
        pressure: None,
        time_ms: t,
    });
}

fn scroll(e: &mut Editor, dy: f64, at: V2) {
    e.handle(InputEvent::Scroll {
        delta: v2(0.0, dy),
        pos: at,
    });
}

fn press(e: &mut Editor, k: Key) {
    e.handle(InputEvent::KeyDown {
        key: k,
        repeat: false,
    });
}

/// Which frame is showing.
fn showing(e: &Editor) -> NodeId {
    e.preview.as_ref().unwrap().current.frame
}

fn scroll_y(e: &Editor) -> f64 {
    e.preview.as_ref().unwrap().current.scroll.y
}

// ---------------------------------------------------------------------------------- navigation

#[test]
fn a_link_opens_a_screen_on_another_page_and_leaves_the_editor_alone() {
    let mut s = site();
    let before_doc = s.e.doc.clone();
    let before_about = s.e.page_doc(1).clone();
    let before_camera = s.e.camera;
    s.e.selection = vec![s.button];
    let before_history = s.e.history.generation();
    let dirty = s.e.project_dirty();
    start(&mut s);

    // The frame fits the window exactly, so design px are screen px here.
    click(&mut s.e, v2(150.0, 120.0), 10.0);
    assert_eq!(showing(&s.e), s.about);
    assert_eq!(
        s.e.preview.as_ref().unwrap().current.page,
        s.about_page,
        "the screen is on the other page"
    );

    assert_eq!(s.e.active_page, 0, "the editor stays on its page");
    assert_eq!(s.e.doc, before_doc);
    assert_eq!(s.e.page_doc(1), &before_about);
    assert_eq!(s.e.camera, before_camera);
    assert_eq!(s.e.selection, vec![s.button]);
    assert_eq!(s.e.history.generation(), before_history);
    assert_eq!(s.e.project_dirty(), dirty, "previewing never needs a save");

    s.e.stop_preview();
    assert!(s.e.preview.is_none());
    assert_eq!(s.e.doc, before_doc);
}

#[test]
fn back_returns_to_the_screen_where_you_left_it() {
    let mut s = site();
    start(&mut s);
    scroll(&mut s.e, 90.0, v2(500.0, 300.0));
    assert_eq!(scroll_y(&s.e), 90.0);
    // The page moved up, so the button is 90px higher on screen.
    click(&mut s.e, v2(150.0, 30.0), 10.0);
    assert_eq!(showing(&s.e), s.about);
    assert_eq!(scroll_y(&s.e), 0.0, "a new screen starts at the top");
    assert!(s.e.preview.as_ref().unwrap().can_go_back());

    click(&mut s.e, v2(50.0, 30.0), 20.0);
    assert_eq!(showing(&s.e), s.home, "the Back layer goes back");
    assert_eq!(scroll_y(&s.e), 90.0, "scroll is restored");
    assert!(!s.e.preview.as_ref().unwrap().can_go_back());
}

#[test]
fn a_link_to_a_layer_inside_a_frame_opens_that_screen_scrolled_to_it() {
    let mut s = site();
    let target = Target {
        page: Some(s.about_page.clone()),
        node: s.section,
    };
    set_interactions(&mut s.e, s.button, vec![click_to(target)]);
    start(&mut s);
    click(&mut s.e, v2(150.0, 120.0), 10.0);
    assert_eq!(showing(&s.e), s.about);
    assert_eq!(scroll_y(&s.e), 1500.0);
}

#[test]
fn a_section_link_on_the_same_screen_scrolls_smoothly_and_clears_pinned_headers() {
    let mut s = site();
    let (home, button) = (s.home, s.button);
    let header = s.e.add_node(
        box_node(Rect::new(0.0, 0.0, 1000.0, 80.0), BLUE),
        Some(home),
    );
    s.e.edit("pin", |tx| {
        tx.node_mut(header)
            .unwrap()
            .edit_proto(|p| p.position = ScrollPosition::Fixed);
    });
    let lower = s.e.add_node(
        box_node(Rect::new(0.0, 1000.0, 1000.0, 100.0), GREEN),
        Some(home),
    );
    set_interactions(&mut s.e, button, vec![click_to(Target::same_page(lower))]);
    start(&mut s);
    click(&mut s.e, v2(150.0, 120.0), 10.0);
    assert_eq!(showing(&s.e), home, "same screen: no navigation");
    assert!(!s.e.preview.as_ref().unwrap().can_go_back());
    s.e.tick(170.0);
    let midway = scroll_y(&s.e);
    assert!(midway > 0.0 && midway < 920.0, "still gliding: {midway}");
    s.e.tick(1000.0);
    assert_eq!(scroll_y(&s.e), 920.0, "stops below the 80px header");
}

#[test]
fn scrolling_stops_at_both_ends_of_the_page() {
    let mut s = site();
    start(&mut s);
    scroll(&mut s.e, -50.0, v2(500.0, 300.0));
    assert_eq!(scroll_y(&s.e), 0.0);
    scroll(&mut s.e, 1_000_000.0, v2(500.0, 300.0));
    assert_eq!(scroll_y(&s.e), 2000.0 - VIEW_H);
    press(&mut s.e, Key::Home);
    s.e.tick(5000.0);
    assert_eq!(scroll_y(&s.e), 0.0);
    press(&mut s.e, Key::End);
    s.e.tick(10000.0);
    assert_eq!(scroll_y(&s.e), 2000.0 - VIEW_H);
}

#[test]
fn a_window_narrower_than_the_frame_scales_it_and_clicks_still_land() {
    let mut s = site();
    s.e.viewport = v2(500.0, 400.0);
    start(&mut s);
    // At half scale the button's (100, 100) is at (50, 50) on screen.
    let hit = s.e.preview.as_ref().unwrap().pick(v2(75.0, 60.0));
    assert!(
        matches!(&hit, Hit::Layer { chain, .. } if chain[0] == s.button),
        "{hit:?}"
    );
    click(&mut s.e, v2(75.0, 60.0), 10.0);
    assert_eq!(showing(&s.e), s.about);
}

#[test]
fn a_narrow_frame_is_centered_with_the_stage_around_it() {
    let mut s = site();
    let phone =
        s.e.add_node(frame_node(Rect::new(0.0, 0.0, 400.0, 800.0), WHITE), None);
    let tap = s.e.add_node(
        box_node(Rect::new(0.0, 0.0, 400.0, 100.0), RED),
        Some(phone),
    );
    assert!(s.e.start_preview(Some((0, phone))));
    let p = s.e.preview.as_ref().unwrap();
    assert!(matches!(&p.pick(v2(310.0, 50.0)), Hit::Layer { chain, .. } if chain[0] == tap));
    assert_eq!(p.pick(v2(100.0, 50.0)), Hit::Stage);
}

#[test]
fn a_click_only_counts_when_released_on_the_same_layer() {
    let mut s = site();
    start(&mut s);
    down(&mut s.e, v2(150.0, 120.0), 10.0);
    up(&mut s.e, v2(800.0, 500.0), 20.0);
    assert_eq!(showing(&s.e), s.home);
    down(&mut s.e, v2(800.0, 500.0), 30.0);
    up(&mut s.e, v2(150.0, 120.0), 40.0);
    assert_eq!(showing(&s.e), s.home);
    down(&mut s.e, v2(150.0, 120.0), 50.0);
    up(&mut s.e, v2(160.0, 125.0), 60.0);
    assert_eq!(showing(&s.e), s.about);
}

#[test]
fn a_click_inside_a_linked_card_bubbles_up_to_it() {
    let mut s = site();
    let home = s.home;
    let card = s.e.add_node(
        frame_node(Rect::new(400.0, 100.0, 300.0, 200.0), YELLOW),
        Some(home),
    );
    let inner = s.e.add_node(
        box_node(Rect::new(20.0, 20.0, 50.0, 50.0), GREEN),
        Some(card),
    );
    let target = Target {
        page: Some(s.about_page.clone()),
        node: s.about,
    };
    set_interactions(&mut s.e, card, vec![click_to(target)]);
    start(&mut s);
    let p = s.e.preview.as_ref().unwrap();
    assert!(matches!(&p.pick(v2(430.0, 130.0)), Hit::Layer { chain, .. } if chain[0] == inner));
    click(&mut s.e, v2(430.0, 130.0), 10.0);
    assert_eq!(showing(&s.e), s.about);
}

#[test]
fn empty_layout_frames_and_lines_on_top_do_not_block_a_button() {
    let mut s = site();
    let home = s.home;
    let mut clear = frame_node(Rect::new(0.0, 0.0, 1000.0, 400.0), WHITE);
    clear.style.fills.clear();
    s.e.add_node(clear, Some(home));
    s.e.add_node(
        Node::new(
            NodeId(0),
            NodeKind::Line {
                a: V2::ZERO,
                b: v2(500.0, 300.0),
            },
            Rect::new(0.0, 0.0, 500.0, 300.0),
        ),
        Some(home),
    );
    start(&mut s);
    click(&mut s.e, v2(150.0, 120.0), 10.0);
    assert_eq!(showing(&s.e), s.about);
}

#[test]
fn a_painted_layer_on_top_does_block_what_is_under_it() {
    let mut s = site();
    let home = s.home;
    s.e.add_node(
        box_node(Rect::new(0.0, 0.0, 1000.0, 400.0), GREEN),
        Some(home),
    );
    start(&mut s);
    click(&mut s.e, v2(150.0, 120.0), 10.0);
    assert_eq!(showing(&s.e), s.home);
}

#[test]
fn restart_returns_to_the_first_screen_and_forgets_history() {
    let mut s = site();
    start(&mut s);
    click(&mut s.e, v2(150.0, 120.0), 10.0);
    assert_eq!(showing(&s.e), s.about);
    press(&mut s.e, Key::Char('r'));
    assert_eq!(showing(&s.e), s.home);
    assert!(!s.e.preview.as_ref().unwrap().can_go_back());
}

// ---------------------------------------------------------------------------------- states

#[test]
fn hover_style_shows_only_in_the_preview_and_follows_the_pointer() {
    let mut s = site();
    let button = s.button;
    s.e.edit("hover", |tx| {
        tx.node_mut(button).unwrap().edit_proto(|p| {
            p.hover = Some(StateStyle {
                fill: Some(BLUE),
                offset: Some(v2(0.0, -4.0)),
                ..StateStyle::default()
            });
        });
    });
    start(&mut s);
    let page = s.e.pages[0].id.clone();
    let fill = |e: &Editor| {
        e.preview
            .as_ref()
            .unwrap()
            .doc(&page)
            .unwrap()
            .get(button)
            .unwrap()
            .style
            .fill()
    };
    assert_eq!(fill(&s.e), Some(RED));
    hover(&mut s.e, v2(150.0, 120.0), 10.0);
    assert_eq!(fill(&s.e), Some(BLUE));
    assert!(s.e.preview.as_ref().unwrap().cursor_hand);
    assert_eq!(
        s.e.doc.get(button).unwrap().style.fill(),
        Some(RED),
        "not the document"
    );
    hover(&mut s.e, v2(800.0, 500.0), 20.0);
    assert_eq!(fill(&s.e), Some(RED));
    assert!(!s.e.preview.as_ref().unwrap().cursor_hand);
}

#[test]
fn hovering_anything_inside_a_card_applies_its_hover_and_recolors_its_text() {
    let mut s = site();
    let home = s.home;
    let card = s.e.add_node(
        frame_node(Rect::new(400.0, 100.0, 300.0, 200.0), YELLOW),
        Some(home),
    );
    let label = s.e.add_node(
        sketch_rs::demo::text_node("Read more", 20.0, v2(20.0, 20.0), Color::BLACK),
        Some(card),
    );
    s.e.edit("hover", |tx| {
        tx.node_mut(card).unwrap().edit_proto(|p| {
            p.hover = Some(StateStyle {
                fill: Some(GREEN),
                text: Some(WHITE),
                ..StateStyle::default()
            });
        });
    });
    start(&mut s);
    hover(&mut s.e, v2(430.0, 130.0), 10.0);
    let page = s.e.pages[0].id.clone();
    let p = s.e.preview.as_ref().unwrap();
    let doc = p.doc(&page).unwrap();
    assert_eq!(doc.get(card).unwrap().style.fill(), Some(GREEN));
    assert_eq!(doc.get(label).unwrap().style.fill(), Some(WHITE));
}

#[test]
fn pressed_style_lasts_until_release() {
    let mut s = site();
    let button = s.button;
    s.e.edit("pressed", |tx| {
        tx.node_mut(button).unwrap().edit_proto(|p| {
            p.interactions.clear();
            p.pressed = Some(StateStyle {
                opacity: Some(0.5),
                ..StateStyle::default()
            });
        });
    });
    start(&mut s);
    let page = s.e.pages[0].id.clone();
    let opacity = |e: &Editor| {
        e.preview
            .as_ref()
            .unwrap()
            .doc(&page)
            .unwrap()
            .get(button)
            .unwrap()
            .style
            .opacity
    };
    hover(&mut s.e, v2(150.0, 120.0), 5.0);
    down(&mut s.e, v2(150.0, 120.0), 10.0);
    assert_eq!(opacity(&s.e), 0.5);
    up(&mut s.e, v2(150.0, 120.0), 20.0);
    assert_eq!(opacity(&s.e), 1.0);
}

// ---------------------------------------------------------------------------------- overlays

fn overlay_site(hover_trigger: bool, placement: OverlayPlacement) -> (Site, NodeId) {
    let mut s = site();
    let menu = s.e.add_node(
        frame_node(Rect::new(2000.0, 0.0, 300.0, 200.0), YELLOW),
        None,
    );
    let action = ProtoAction::OpenOverlay {
        target: Target::same_page(menu),
        placement,
        dim: true,
        close_outside: true,
        transition: Transition::default(),
    };
    set_interactions(
        &mut s.e,
        s.button,
        vec![Interaction {
            trigger: if hover_trigger {
                Trigger::Hover
            } else {
                Trigger::Click
            },
            action,
        }],
    );
    (s, menu)
}

#[test]
fn an_overlay_opens_over_the_screen_and_a_click_outside_closes_it() {
    let (mut s, menu) = overlay_site(false, OverlayPlacement::Center);
    start(&mut s);
    click(&mut s.e, v2(150.0, 120.0), 10.0);
    let p = s.e.preview.as_ref().unwrap();
    assert_eq!(p.overlays.len(), 1);
    assert_eq!(p.overlays[0].screen.frame, menu);
    assert_eq!(p.current.frame, s.home, "the screen stays under it");
    // Centered: 300×200 in 1000×600 is (350, 200).
    assert!(matches!(
        &p.pick(v2(500.0, 300.0)),
        Hit::Layer {
            overlay: Some(0),
            ..
        }
    ));
    assert_eq!(
        p.pick(v2(150.0, 120.0)),
        Hit::Backdrop(0),
        "the button is behind it"
    );

    click(&mut s.e, v2(500.0, 300.0), 20.0);
    assert_eq!(
        s.e.preview.as_ref().unwrap().overlays.len(),
        1,
        "inside stays open"
    );
    click(&mut s.e, v2(150.0, 120.0), 30.0);
    assert_eq!(s.e.preview.as_ref().unwrap().overlays.len(), 0);
    assert_eq!(showing(&s.e), s.home, "that click didn't reach the button");
}

#[test]
fn escape_closes_an_overlay_then_leaves_the_preview() {
    let (mut s, _) = overlay_site(false, OverlayPlacement::Center);
    start(&mut s);
    click(&mut s.e, v2(150.0, 120.0), 10.0);
    press(&mut s.e, Key::Escape);
    assert_eq!(s.e.preview.as_ref().unwrap().overlays.len(), 0);
    press(&mut s.e, Key::Escape);
    assert!(s.e.preview.is_none());
}

#[test]
fn back_closes_an_overlay_before_leaving_the_screen() {
    let (mut s, _) = overlay_site(false, OverlayPlacement::Center);
    start(&mut s);
    click(&mut s.e, v2(150.0, 120.0), 10.0);
    press(&mut s.e, Key::Left);
    let p = s.e.preview.as_ref().unwrap();
    assert_eq!(p.overlays.len(), 0);
    assert_eq!(p.current.frame, s.home);
}

#[test]
fn a_hover_dropdown_stays_while_moving_onto_it_and_closes_after_leaving_both() {
    let (mut s, _) = overlay_site(true, OverlayPlacement::Below);
    start(&mut s);
    hover(&mut s.e, v2(150.0, 120.0), 10.0);
    let p = s.e.preview.as_ref().unwrap();
    assert_eq!(p.overlays.len(), 1);
    // Hangs from the bottom-left of the 100,100 200×50 button.
    assert!(matches!(
        &p.pick(v2(105.0, 155.0)),
        Hit::Layer {
            overlay: Some(0),
            ..
        }
    ));
    assert!(
        matches!(&p.pick(v2(800.0, 500.0)), Hit::Layer { overlay: None, .. }),
        "a hover overlay doesn't block the page"
    );

    // Off the button, across the gap, and onto the menu in time.
    hover(&mut s.e, v2(150.0, 160.0), 20.0);
    s.e.tick(40.0);
    assert_eq!(s.e.preview.as_ref().unwrap().overlays.len(), 1);
    s.e.tick(5000.0);
    assert_eq!(
        s.e.preview.as_ref().unwrap().overlays.len(),
        1,
        "it's on the menu"
    );

    hover(&mut s.e, v2(800.0, 500.0), 5100.0);
    s.e.tick(5150.0);
    assert_eq!(
        s.e.preview.as_ref().unwrap().overlays.len(),
        1,
        "grace period"
    );
    s.e.tick(5400.0);
    assert_eq!(s.e.preview.as_ref().unwrap().overlays.len(), 0);
}

// ---------------------------------------------------------------------------------- timers, links

#[test]
fn a_delayed_action_runs_once_and_is_cancelled_by_leaving() {
    let mut s = site();
    let (home, about_page, about) = (s.home, s.about_page.clone(), s.about);
    set_interactions(
        &mut s.e,
        home,
        vec![Interaction {
            trigger: Trigger::Delay { ms: 1000.0 },
            action: ProtoAction::Navigate {
                target: Target {
                    page: Some(about_page),
                    node: about,
                },
                transition: Transition::default(),
            },
        }],
    );
    start(&mut s);
    assert_eq!(
        s.e.preview.as_ref().unwrap().next_wake(0.0),
        Some(1000.0),
        "the window sleeps until then"
    );
    s.e.tick(999.0);
    assert_eq!(showing(&s.e), home);
    s.e.tick(1001.0);
    assert_eq!(showing(&s.e), about);
    assert_eq!(s.e.preview.as_ref().unwrap().next_wake(1001.0), None);
}

#[test]
fn links_leave_the_app_only_for_web_addresses() {
    let mut s = site();
    let url = |u: &str| Interaction {
        trigger: Trigger::Click,
        action: ProtoAction::OpenUrl { url: u.into() },
    };
    set_interactions(&mut s.e, s.button, vec![url("https://example.com/a")]);
    start(&mut s);
    click(&mut s.e, v2(150.0, 120.0), 10.0);
    assert_eq!(
        s.e.requests,
        vec![AppRequest::OpenUrl("https://example.com/a".into())]
    );
    s.e.requests.clear();
    s.e.stop_preview();

    set_interactions(&mut s.e, s.button, vec![url("file:///etc/passwd")]);
    start(&mut s);
    click(&mut s.e, v2(150.0, 120.0), 20.0);
    assert!(s.e.requests.is_empty(), "{:?}", s.e.requests);
    assert!(s.e.status.contains("Not opening"), "{}", s.e.status);
}

#[test]
fn a_link_to_something_deleted_does_nothing() {
    let mut s = site();
    let (i, about) = (1, s.about);
    s.e.with_page(i, |e| {
        e.edit("delete", |tx| tx.remove(about));
    });
    start(&mut s);
    click(&mut s.e, v2(150.0, 120.0), 10.0);
    assert_eq!(showing(&s.e), s.home);
}

// ---------------------------------------------------------------------------------- editor safety

#[test]
fn editing_shortcuts_do_nothing_while_previewing() {
    let mut s = site();
    let (button, before) = (s.button, s.e.doc.clone());
    s.e.selection = vec![button];
    start(&mut s);
    for a in [
        Action::Delete,
        Action::Undo,
        Action::Duplicate,
        Action::SelectAll,
        Action::Nudge(5.0, 5.0),
        Action::Group,
        Action::NextPage,
    ] {
        s.e.run(a);
    }
    key(&mut s.e, Key::Delete, Modifiers::NONE);
    key(&mut s.e, Key::Char('z'), Modifiers::cmd());
    key(&mut s.e, Key::Char('d'), Modifiers::cmd());
    assert_eq!(s.e.doc, before);
    assert_eq!(s.e.active_page, 0);
    assert!(s.e.preview.is_some());
}

#[test]
fn the_shortcut_toggles_the_preview_and_the_panel() {
    let mut s = site();
    key(&mut s.e, Key::Char('e'), Modifiers::SHIFT);
    assert!(s.e.proto_mode);
    key(&mut s.e, Key::Char('e'), Modifiers::SHIFT);
    assert!(!s.e.proto_mode);

    let chord = Modifiers {
        alt: true,
        ..Modifiers::cmd()
    };
    key(&mut s.e, Key::Enter, chord);
    assert!(s.e.preview.is_some());
    key(&mut s.e, Key::Enter, chord);
    assert!(s.e.preview.is_none());
}

#[test]
fn starting_uses_the_selection_then_the_view() {
    let mut s = site();
    let other = s.e.add_node(
        frame_node(Rect::new(5000.0, 0.0, 400.0, 400.0), WHITE),
        None,
    );
    s.e.selection = vec![s.button];
    s.e.run(Action::TogglePreview);
    assert_eq!(showing(&s.e), s.home, "the selected layer's frame");
    s.e.stop_preview();

    s.e.selection.clear();
    let middle = s.e.viewport * 0.5;
    s.e.camera.pan = v2(5200.0, 200.0) - middle;
    s.e.run(Action::TogglePreview);
    assert_eq!(showing(&s.e), other, "the frame in the middle of the view");
}

#[test]
fn a_page_with_no_frames_says_so_instead_of_starting() {
    let mut e = editor(VIEW_W, VIEW_H);
    e.run(Action::TogglePreview);
    assert!(e.preview.is_none());
    assert!(e.status.contains("no frames"), "{}", e.status);
}

#[test]
fn edits_made_while_previewing_show_up_and_deleting_the_screen_ends_it() {
    let mut s = site();
    let (home, button) = (s.home, s.button);
    start(&mut s);
    // An agent recolors the button over MCP while the user watches.
    s.e.edit("recolor", |tx| {
        tx.node_mut(button).unwrap().style.set_fill(Some(GREEN));
    });
    s.e.preview_sync();
    let page = s.e.pages[0].id.clone();
    let seen = s.e.preview.as_ref().unwrap().doc(&page).unwrap();
    assert_eq!(seen.get(button).unwrap().style.fill(), Some(GREEN));

    s.e.edit("delete", |tx| tx.remove(home));
    s.e.preview_sync();
    assert!(s.e.preview.is_none());
    assert!(s.e.status.contains("deleted"), "{}", s.e.status);
}

// ---------------------------------------------------------------------------------- copying

fn link_target(e: &Editor, id: NodeId) -> Target {
    e.doc
        .get(id)
        .unwrap()
        .interactions()
        .first()
        .unwrap()
        .action
        .target()
        .unwrap()
        .clone()
}

#[test]
fn copies_of_a_screen_and_its_link_target_link_to_each_other() {
    let mut e = editor(VIEW_W, VIEW_H);
    let a = e.add_node(frame_node(Rect::new(0.0, 0.0, 100.0, 100.0), WHITE), None);
    let b = e.add_node(frame_node(Rect::new(200.0, 0.0, 100.0, 100.0), WHITE), None);
    set_interactions(&mut e, a, vec![click_to(Target::same_page(b))]);
    e.selection = vec![a, b];
    e.run(Action::Duplicate);
    let copies = e.selection.clone();
    assert_eq!(copies.len(), 2);
    let t = link_target(&e, copies[0]);
    assert_eq!(t.node, copies[1], "the copy links to the copy");
    assert_eq!(t.page, None);
}

#[test]
fn a_layer_copied_to_another_page_keeps_linking_to_where_it_pointed() {
    let mut s = site();
    let home_page = s.e.pages[0].id.clone();
    let home = s.home;
    let link =
        s.e.add_node(box_node(Rect::new(0.0, 0.0, 50.0, 50.0), RED), Some(home));
    set_interactions(&mut s.e, link, vec![click_to(Target::same_page(home))]);
    s.e.selection = vec![link];
    s.e.run(Action::Copy);
    s.e.switch_page(1);
    s.e.run(Action::Paste);
    let copy = s.e.selection[0];
    let t = link_target(&s.e, copy);
    assert_eq!(t.page.as_deref(), Some(home_page.as_str()));
    assert_eq!(t.node, home);
    let about_id = s.e.pages[1].id.clone();
    assert!(s.e.resolve_target(&about_id, &t).is_some());
}

#[test]
fn links_inside_a_duplicated_page_stay_inside_it() {
    let mut s = site();
    let (home, button) = (s.home, s.button);
    set_interactions(&mut s.e, button, vec![click_to(Target::same_page(home))]);
    let dup = s.e.duplicate_page(0);
    assert_eq!(s.e.active_page, dup);
    let t = link_target(&s.e, button);
    assert_eq!(t.page, None);
    let dup_id = s.e.pages[dup].id.clone();
    assert_eq!(s.e.resolve_target(&dup_id, &t).unwrap().page_index, dup);
}

#[test]
fn undo_takes_a_link_back() {
    let mut s = site();
    let button = s.button;
    assert!(!s.e.doc.get(button).unwrap().interactions().is_empty());
    s.e.undo();
    assert!(s.e.doc.get(button).unwrap().interactions().is_empty());
    s.e.redo();
    assert!(!s.e.doc.get(button).unwrap().interactions().is_empty());
}

#[test]
fn instances_follow_their_component_until_they_override_it() {
    let mut e = editor(VIEW_W, VIEW_H);
    let target = e.add_node(frame_node(Rect::new(800.0, 0.0, 100.0, 100.0), WHITE), None);
    let other = e.add_node(
        frame_node(Rect::new(800.0, 200.0, 100.0, 100.0), WHITE),
        None,
    );
    let main = e.add_node(frame_node(Rect::new(0.0, 0.0, 100.0, 40.0), RED), None);
    e.selection = vec![main];
    e.create_component();
    let inst = e.create_instance(main, v2(0.0, 300.0)).unwrap();

    set_interactions(&mut e, main, vec![click_to(Target::same_page(target))]);
    e.commit();
    assert_eq!(link_target(&e, inst).node, target, "follows the component");

    e.edit("own link", |tx| {
        tx.node_mut(inst)
            .unwrap()
            .edit_proto(|p| p.interactions = vec![click_to(Target::same_page(other))]);
    });
    e.mark_overrides(&[inst], "prototype");
    e.commit();
    set_interactions(&mut e, main, vec![click_to(Target::same_page(target))]);
    e.edit("touch", |tx| {
        tx.node_mut(main).unwrap().name = "Button".into()
    });
    assert_eq!(link_target(&e, inst).node, other, "its own link survives");
}

// ---------------------------------------------------------------------------------- drawing links

#[test]
fn dragging_the_knob_onto_a_frame_links_to_it_and_undoes_in_one_step() {
    let mut e = editor(VIEW_W, VIEW_H);
    let a = e.add_node(frame_node(Rect::new(0.0, 0.0, 300.0, 300.0), WHITE), None);
    let btn = e.add_node(box_node(Rect::new(20.0, 20.0, 100.0, 40.0), RED), Some(a));
    let b = e.add_node(frame_node(Rect::new(500.0, 0.0, 300.0, 300.0), WHITE), None);
    e.selection = vec![btn];
    assert_eq!(e.proto_knob(), None, "only with the Prototype panel open");
    e.proto_mode = true;
    let knob = e.proto_knob().unwrap();

    ev_drag(&mut e, knob, v2(650.0, 150.0), 0.0, false);
    assert_eq!(
        e.bind_hover,
        Some(b),
        "the frame under the pointer lights up"
    );
    assert!(e.doc.get(btn).unwrap().interactions().is_empty());
    up(&mut e, v2(650.0, 150.0), 300.0);

    assert_eq!(link_target(&e, btn), Target::same_page(b));
    assert_eq!(e.bind_hover, None);
    assert_eq!(
        e.doc.world_rect(btn).unwrap().x,
        20.0,
        "the layer didn't move"
    );
    e.undo();
    assert!(e.doc.get(btn).unwrap().interactions().is_empty());
}

#[test]
fn dropping_inside_its_own_screen_links_to_that_section() {
    let mut e = editor(VIEW_W, VIEW_H);
    let a = e.add_node(frame_node(Rect::new(0.0, 0.0, 600.0, 500.0), WHITE), None);
    let btn = e.add_node(box_node(Rect::new(20.0, 20.0, 100.0, 40.0), RED), Some(a));
    let section = e.add_node(
        frame_node(Rect::new(0.0, 300.0, 600.0, 200.0), GREEN),
        Some(a),
    );
    e.selection = vec![btn];
    e.proto_mode = true;
    let knob = e.proto_knob().unwrap();
    ev_drag(&mut e, knob, v2(300.0, 400.0), 0.0, true);
    assert_eq!(link_target(&e, btn), Target::same_page(section));
}

#[test]
fn a_new_link_replaces_the_old_one_and_dropping_on_nothing_changes_nothing() {
    let mut e = editor(VIEW_W, VIEW_H);
    let a = e.add_node(frame_node(Rect::new(0.0, 0.0, 300.0, 300.0), WHITE), None);
    let btn = e.add_node(box_node(Rect::new(20.0, 20.0, 100.0, 40.0), RED), Some(a));
    let b = e.add_node(frame_node(Rect::new(500.0, 0.0, 300.0, 300.0), WHITE), None);
    set_interactions(&mut e, btn, vec![click_to(Target::same_page(a))]);
    e.selection = vec![btn];
    e.proto_mode = true;
    let knob = e.proto_knob().unwrap();

    let gen_before = e.history.generation();
    ev_drag(&mut e, knob, v2(450.0, 550.0), 0.0, true);
    assert_eq!(e.history.generation(), gen_before);
    assert_eq!(link_target(&e, btn), Target::same_page(a));

    ev_drag(&mut e, knob, v2(650.0, 150.0), 300.0, true);
    assert_eq!(e.doc.get(btn).unwrap().interactions().len(), 1);
    assert_eq!(link_target(&e, btn), Target::same_page(b));
}

// ---------------------------------------------------------------------------------- MCP

fn host() -> exec::SessionHost {
    let root = std::env::temp_dir().join(format!("sketch-rs-proto-{:016x}", fastrand::u64(..)));
    let mut h = exec::SessionHost::new(Store::open(root).unwrap());
    h.session.create(&mut h.ed, "Prototype test").unwrap();
    h
}

fn try_call(h: &mut exec::SessionHost, name: &str, args: Value) -> Result<Value, String> {
    exec::call(h, name, &args).map(|outs| match outs.into_iter().next() {
        Some(Out::Text(t)) => serde_json::from_str(&t).unwrap_or(Value::String(t)),
        _ => Value::Null,
    })
}

fn call(h: &mut exec::SessionHost, name: &str, args: Value) -> Value {
    try_call(h, name, args.clone()).unwrap_or_else(|e| panic!("{name} {args}: {e}"))
}

fn artboard(h: &mut exec::SessionHost, name: &str, w: u32, page: Option<&str>) -> String {
    let mut args = json!({
        "name": name,
        "styles": {"width": format!("{w}px"), "height": "900px"},
    });
    if let Some(p) = page {
        args["pageId"] = json!(p);
    }
    call(h, "create_artboard", args)["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A home page and an About page, each with a desktop and a phone screen.
fn mcp_site() -> (exec::SessionHost, [String; 4]) {
    let mut h = host();
    let home_d = artboard(&mut h, "Home Desktop", 1440, None);
    let home_m = artboard(&mut h, "Home Mobile", 390, None);
    let about = call(&mut h, "create_page", json!({"name": "About"}))["pageId"]
        .as_str()
        .unwrap()
        .to_string();
    let about_d = artboard(&mut h, "About Desktop", 1440, Some(&about));
    let about_m = artboard(&mut h, "About Mobile", 390, Some(&about));
    (h, [home_d, home_m, about_d, about_m])
}

#[test]
fn set_interactions_links_by_id_and_by_name_across_pages() {
    let (mut h, [home_d, home_m, about_d, about_m]) = mcp_site();
    let r = call(
        &mut h,
        "set_interactions",
        json!({"updates": [
            {"nodeId": home_d, "interactions": [
                {"action": "navigate", "target": about_d, "transition": {"type": "push", "direction": "up", "duration": 200}},
            ], "hover": {"backgroundColor": "#112233", "color": "#ffffff", "transform": "translateY(-2px)"}},
            // A page name picks the frame closest in width to the link's own frame.
            {"nodeId": home_m, "interactions": [{"action": "navigate", "target": "About"}]},
        ]}),
    );
    assert_eq!(r["updated"], 2);
    let n = |id: &str| id.rsplit(':').next().unwrap().parse::<u64>().unwrap();
    let doc = h.ed.page_doc(0);
    let d = doc.get(NodeId(n(&home_d))).unwrap();
    match &d.interactions()[0].action {
        ProtoAction::Navigate { target, transition } => {
            assert_eq!(target.node, NodeId(n(&about_d)));
            assert_eq!(transition.kind, TransitionKind::Push);
            assert_eq!(transition.dir, SlideDir::Up);
            assert_eq!(transition.ms, 200.0);
        }
        o => panic!("{o:?}"),
    }
    let hover = d.proto.as_ref().unwrap().hover.unwrap();
    assert_eq!(hover.fill, Some(Color::rgb(0x11, 0x22, 0x33)));
    assert_eq!(hover.text, Some(WHITE));
    assert_eq!(hover.offset, Some(v2(0.0, -2.0)));
    let m = doc.get(NodeId(n(&home_m))).unwrap();
    assert_eq!(
        m.interactions()[0].action.target().unwrap().node,
        NodeId(n(&about_m)),
        "a phone screen's link to a page reaches that page's phone screen"
    );
    // Undo on that page takes the whole call back in one step.
    let home_page = h.ed.pages[0].id.clone();
    call(&mut h, "undo", json!({"pageId": home_page}));
    assert!(
        h.ed.page_doc(0)
            .get(NodeId(n(&home_d)))
            .unwrap()
            .proto
            .is_none()
    );
}

#[test]
fn set_interactions_rejects_a_bad_update_without_applying_any() {
    let (mut h, [home_d, home_m, ..]) = mcp_site();
    for bad in [
        json!({"action": "navigate", "target": "Nowhere"}),
        json!({"action": "open_url", "url": "file:///etc/passwd"}),
        json!({"action": "teleport"}),
        json!({"action": "navigate"}),
        json!({"trigger": "drag", "action": "back"}),
    ] {
        let e = try_call(
            &mut h,
            "set_interactions",
            json!({"updates": [
                {"nodeId": home_m, "interactions": [{"action": "back"}]},
                {"nodeId": home_d, "interactions": [bad.clone()]},
            ]}),
        );
        assert!(e.is_err(), "{bad}");
    }
    let doc = h.ed.page_doc(0);
    assert!(doc.nodes.values().all(|n| n.proto.is_none()));
}

#[test]
fn set_interactions_changes_only_what_it_names() {
    let (mut h, [home_d, ..]) = mcp_site();
    call(
        &mut h,
        "set_interactions",
        json!({"updates": [{"nodeId": home_d, "interactions": [{"action": "back"}], "hover": {"opacity": 0.5}}]}),
    );
    call(
        &mut h,
        "set_interactions",
        json!({"updates": [{"nodeId": home_d, "position": "sticky"}]}),
    );
    let n = home_d.rsplit(':').next().unwrap().parse::<u64>().unwrap();
    let p = h.ed.doc.get(NodeId(n)).unwrap().proto.clone().unwrap();
    assert_eq!(p.interactions.len(), 1);
    assert_eq!(p.hover.unwrap().opacity, Some(0.5));
    assert_eq!(p.position, ScrollPosition::Sticky);

    call(
        &mut h,
        "set_interactions",
        json!({"updates": [{"nodeId": home_d, "interactions": [], "hover": null, "position": "scroll"}]}),
    );
    assert!(h.ed.doc.get(NodeId(n)).unwrap().proto.is_none());
}

#[test]
fn the_preview_tool_follows_links_for_an_agent() {
    let (mut h, [home_d, _, about_d, _]) = mcp_site();
    call(
        &mut h,
        "set_interactions",
        json!({"updates": [{"nodeId": home_d, "interactions": [{"action": "navigate", "target": about_d}]}]}),
    );
    assert_eq!(
        call(&mut h, "preview", json!({"action": "status"}))["running"],
        false
    );
    assert!(
        try_call(
            &mut h,
            "preview",
            json!({"action": "click", "nodeId": home_d})
        )
        .is_err()
    );

    let r = call(
        &mut h,
        "preview",
        json!({"action": "start", "nodeId": home_d}),
    );
    assert_eq!(r["screen"], "Home Desktop");
    let r = call(
        &mut h,
        "preview",
        json!({"action": "click", "nodeId": home_d}),
    );
    assert_eq!(r["screen"], "About Desktop");
    assert_eq!(r["page"], "About");
    assert_eq!(r["canGoBack"], true);
    let r = call(&mut h, "preview", json!({"action": "back"}));
    assert_eq!(r["screen"], "Home Desktop");
    let r = call(&mut h, "preview", json!({"action": "stop"}));
    assert_eq!(r["running"], false);
}

#[test]
fn clicking_by_position_goes_through_hit_testing_and_reports_what_is_there() {
    let (mut h, [home_d, _, about_d, _]) = mcp_site();
    let r = call(
        &mut h,
        "write_html",
        json!({
            "targetNodeId": home_d, "mode": "insert-children",
            "html": r##"<div layer-name="Cta" style="width:200px; height:50px; background:#ff0000"></div>
                        <div layer-name="Covered" style="width:200px; height:50px; background:#0000ff"></div>
                        <div layer-name="Cover" style="position:absolute; left:0; top:50px; width:200px; height:50px; background:#00ff00"></div>"##,
        }),
    );
    let ids = r["createdNodeIds"].as_array().unwrap().clone();
    call(
        &mut h,
        "set_interactions",
        json!({"updates": [
            {"nodeId": ids[0], "interactions": [{"action": "navigate", "target": about_d}]},
            {"nodeId": ids[1], "interactions": [{"action": "navigate", "target": about_d}]},
        ]}),
    );
    h.ed.viewport = v2(1440.0, 800.0);
    call(
        &mut h,
        "preview",
        json!({"action": "start", "nodeId": home_d}),
    );

    // A layer on top wins, and the agent can see why nothing happened.
    let r = call(
        &mut h,
        "preview",
        json!({"action": "click", "x": 20, "y": 70}),
    );
    assert_eq!(r["under"][0], "Cover", "{r}");
    assert_eq!(r["screen"], "Home Desktop");

    let r = call(
        &mut h,
        "preview",
        json!({"action": "hover", "x": 20, "y": 20}),
    );
    assert_eq!(r["under"][0], "Cta");
    assert_eq!(r["screen"], "Home Desktop", "hovering doesn't click");

    let r = call(
        &mut h,
        "preview",
        json!({"action": "click", "x": 20, "y": 20}),
    );
    assert_eq!(r["screen"], "About Desktop");

    let r = call(&mut h, "preview", json!({"action": "scroll", "y": 60}));
    assert_eq!(r["scrollY"], 60.0);

    if gpu().is_some() {
        let outs = exec::call(&mut h, "preview", &json!({"action": "screenshot"})).unwrap();
        match &outs[0] {
            Out::Image { mime, data } => {
                assert_eq!(mime, "image/png");
                let img = image::load_from_memory(data).unwrap();
                assert_eq!((img.width(), img.height()), (1440, 800));
            }
            _ => panic!("expected an image"),
        }
    }
    call(&mut h, "preview", json!({"action": "stop"}));
    let e = try_call(&mut h, "preview", json!({"action": "screenshot"}));
    assert!(e.is_err());
}

#[test]
fn get_interactions_reports_links_and_flags_ones_going_nowhere() {
    let (mut h, [home_d, home_m, about_d, _]) = mcp_site();
    call(
        &mut h,
        "set_interactions",
        json!({"updates": [
            {"nodeId": home_d, "interactions": [{"action": "navigate", "target": about_d.clone()}]},
            {"nodeId": home_m, "interactions": [{"action": "open_url", "url": "https://a.com"}]},
        ]}),
    );
    let r = call(&mut h, "get_interactions", json!({}));
    let layers = r["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 2);
    let d = layers.iter().find(|l| l["id"] == json!(home_d)).unwrap();
    let t = &d["interactions"][0]["target"];
    assert_eq!(t["page"], "About");
    assert_eq!(t["screen"], "About Desktop");
    assert_eq!(
        call(&mut h, "get_node_info", json!({"nodeId": home_d}))["prototype"]["interactions"][0]["target"]
            ["screen"],
        "About Desktop"
    );

    call(&mut h, "delete_nodes", json!({"nodeIds": [about_d]}));
    let r = call(&mut h, "get_interactions", json!({}));
    let d = r["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == json!(home_d))
        .unwrap()
        .clone();
    assert_eq!(d["interactions"][0]["target"]["missing"], true);
}

#[test]
fn write_html_turns_links_and_states_into_interactions() {
    let (mut h, [home_d, ..]) = mcp_site();
    let r = call(
        &mut h,
        "write_html",
        json!({
            "targetNodeId": home_d, "mode": "insert-children",
            "html": r##"<nav layer-name="Nav" style="display:flex; gap:16px; position:sticky; padding:8px">
                <a layer-name="To about" href="/about.html" data-hover="color:#ff0000">About</a>
                <a layer-name="External" href="https://example.com">Site</a>
                <a layer-name="To section" href="#contact">Contact</a>
                <a layer-name="Bad" href="javascript:alert(1)">Bad</a>
              </nav>
              <div id="contact" layer-name="Contact" style="height:200px"></div>
              <div layer-name="Carousel" style="overflow:auto; height:100px"></div>
              <p layer-name="Copy">Read <a href="https://example.com/more">more</a> here</p>"##,
        }),
    );
    assert!(
        r["warnings"].to_string().contains("javascript:alert(1)"),
        "{r}"
    );
    let by_name = |h: &exec::SessionHost, name: &str| {
        h.ed.doc
            .nodes
            .values()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("no layer {name}"))
            .clone()
    };
    let to_about = by_name(&h, "To about");
    let t = to_about.interactions()[0].action.target().unwrap();
    assert_eq!(t.page.as_deref(), Some(h.ed.pages[1].id.as_str()));
    assert_eq!(
        to_about.proto.as_ref().unwrap().hover.unwrap().text,
        Some(Color::rgb(255, 0, 0))
    );
    assert!(matches!(
        &by_name(&h, "External").interactions()[0].action,
        ProtoAction::OpenUrl { url } if url == "https://example.com"
    ));
    let contact = by_name(&h, "Contact");
    assert_eq!(
        by_name(&h, "To section").interactions()[0]
            .action
            .target()
            .unwrap(),
        &Target::same_page(contact.id)
    );
    assert!(by_name(&h, "Bad").interactions().is_empty());
    assert_eq!(
        by_name(&h, "Nav").proto.as_ref().unwrap().position,
        ScrollPosition::Sticky
    );
    assert_eq!(
        by_name(&h, "Carousel").proto.as_ref().unwrap().overflow,
        Overflow::Both
    );
    assert!(
        matches!(
            &by_name(&h, "Copy").interactions()[0].action,
            ProtoAction::OpenUrl { url } if url == "https://example.com/more"
        ),
        "an inline link can't have its own layer, so its text layer links"
    );
}

#[test]
fn links_and_states_survive_saving_and_reopening_the_project() {
    let (mut h, [home_d, _, about_d, _]) = mcp_site();
    call(
        &mut h,
        "set_interactions",
        json!({"updates": [{"nodeId": home_d, "interactions": [{"action": "navigate", "target": about_d}]}, ]}),
    );
    let before = h.ed.page_doc(0).clone();
    call(&mut h, "finish_working_on_nodes", json!({}));
    let dir = h.session.project.as_ref().unwrap().dir.clone();
    let mut again = exec::SessionHost::new(h.session.store.clone());
    again.session.open(&mut again.ed, &dir).unwrap();
    let i = again
        .ed
        .pages
        .iter()
        .position(|p| p.name == "Page 1")
        .unwrap();
    assert_eq!(again.ed.page_doc(i).nodes, before.nodes);
}

// ---------------------------------------------------------------------------------- GPU

/// One renderer across frames, as in the app, so reuse of GPU buffers can be checked.
struct Screenful<'g> {
    g: &'g Headless,
    r: CanvasRenderer,
    cache: render::PreviewCache,
}

impl<'g> Screenful<'g> {
    fn new(g: &'g Headless) -> Self {
        Screenful {
            g,
            r: CanvasRenderer::new(&g.device, OFFSCREEN_FORMAT),
            cache: Default::default(),
        }
    }

    fn draw(&mut self, e: &mut Editor) -> image::RgbaImage {
        e.preview_sync();
        let f = render::frame(e.preview.as_ref().unwrap(), &mut self.cache, V2::ZERO);
        let size = [(e.viewport.x * SCALE) as u32, (e.viewport.y * SCALE) as u32];
        let tex = self.g.target(size);
        let view = tex.create_view(&Default::default());
        let mut enc = self.g.device.create_command_encoder(&Default::default());
        self.r.render(
            &self.g.device,
            &self.g.queue,
            &mut enc,
            &view,
            size,
            SCALE as f32,
            &f,
            [0.0, 0.0, 0.0, 1.0],
        );
        self.g.queue.submit([enc.finish()]);
        self.g.read(&tex)
    }
}

fn artifact(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".artifacts/prototype");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn a_fixed_header_stays_put_while_the_page_scrolls_and_scrolling_uploads_nothing() {
    let Some(g) = gpu() else { return };
    let mut s = site();
    let home = s.home;
    let header = s.e.add_node(
        box_node(Rect::new(0.0, 0.0, 1000.0, 60.0), BLUE),
        Some(home),
    );
    s.e.edit("fixed", |tx| {
        tx.node_mut(header)
            .unwrap()
            .edit_proto(|p| p.position = ScrollPosition::Fixed);
    });
    // Something lower down, to scroll under the header.
    s.e.add_node(
        box_node(Rect::new(0.0, 300.0, 1000.0, 100.0), GREEN),
        Some(home),
    );
    start(&mut s);
    let mut view = Screenful::new(g);
    let a = view.draw(&mut s.e);
    assert!(close(px(&a, v2(500.0, 30.0)), BLUE, 3), "header");
    assert!(close(px(&a, v2(150.0, 120.0)), RED, 3), "button");
    assert!(close(px(&a, v2(500.0, 350.0)), GREEN, 3), "band");

    let uploads = view.r.uploads;
    scroll(&mut s.e, 300.0, v2(500.0, 300.0));
    let b = view.draw(&mut s.e);
    assert_eq!(view.r.uploads, uploads, "scrolling only moves cameras");
    assert!(close(px(&b, v2(500.0, 30.0)), BLUE, 3), "header stayed");
    assert!(
        close(px(&b, v2(500.0, 80.0)), GREEN, 3),
        "band scrolled up to 50"
    );
    assert!(
        close(px(&b, v2(150.0, 120.0)), WHITE, 3),
        "button scrolled away"
    );

    // Scroll so the band (now 0..100) passes under the 60px header: the header paints on top.
    scroll(&mut s.e, 20.0, v2(500.0, 300.0));
    let c = view.draw(&mut s.e);
    assert!(
        close(px(&c, v2(500.0, 30.0)), BLUE, 3),
        "header over the band"
    );
    assert!(close(px(&c, v2(500.0, 70.0)), GREEN, 3));
}

#[test]
fn a_sticky_layer_scrolls_until_it_reaches_the_top() {
    let Some(g) = gpu() else { return };
    let mut s = site();
    let home = s.home;
    let bar = s.e.add_node(
        box_node(Rect::new(0.0, 200.0, 1000.0, 50.0), BLUE),
        Some(home),
    );
    s.e.edit("sticky", |tx| {
        tx.node_mut(bar)
            .unwrap()
            .edit_proto(|p| p.position = ScrollPosition::Sticky);
    });
    start(&mut s);
    let mut view = Screenful::new(g);
    let a = view.draw(&mut s.e);
    assert!(close(px(&a, v2(500.0, 225.0)), BLUE, 3));
    scroll(&mut s.e, 100.0, v2(500.0, 400.0));
    let b = view.draw(&mut s.e);
    assert!(close(px(&b, v2(500.0, 125.0)), BLUE, 3), "still scrolling");
    assert!(close(px(&b, v2(500.0, 225.0)), WHITE, 3));
    scroll(&mut s.e, 500.0, v2(500.0, 400.0));
    let c = view.draw(&mut s.e);
    assert!(close(px(&c, v2(500.0, 25.0)), BLUE, 3), "stuck at the top");
    assert!(close(px(&c, v2(500.0, 125.0)), WHITE, 3));
}

#[test]
fn clicks_reach_pinned_layers_at_where_they_are_drawn() {
    let mut s = site();
    let home = s.home;
    let header = s.e.add_node(
        box_node(Rect::new(0.0, 0.0, 1000.0, 60.0), BLUE),
        Some(home),
    );
    let target = Target {
        page: Some(s.about_page.clone()),
        node: s.about,
    };
    set_interactions(&mut s.e, header, vec![click_to(target)]);
    s.e.edit("fixed", |tx| {
        tx.node_mut(header)
            .unwrap()
            .edit_proto(|p| p.position = ScrollPosition::Fixed);
    });
    start(&mut s);
    scroll(&mut s.e, 700.0, v2(500.0, 300.0));
    click(&mut s.e, v2(500.0, 30.0), 10.0);
    assert_eq!(showing(&s.e), s.about);
}

#[test]
fn a_scrolling_frame_inside_a_page_takes_the_wheel_first() {
    let Some(g) = gpu() else { return };
    let mut s = site();
    let home = s.home;
    let panel = s.e.add_node(
        frame_node(Rect::new(100.0, 100.0, 300.0, 200.0), WHITE),
        Some(home),
    );
    s.e.add_node(
        box_node(Rect::new(0.0, 0.0, 300.0, 300.0), GREEN),
        Some(panel),
    );
    s.e.add_node(
        box_node(Rect::new(0.0, 300.0, 300.0, 200.0), YELLOW),
        Some(panel),
    );
    s.e.edit("scroll", |tx| {
        tx.node_mut(panel)
            .unwrap()
            .edit_proto(|p| p.overflow = Overflow::Vertical);
    });
    start(&mut s);
    let mut view = Screenful::new(g);
    let a = view.draw(&mut s.e);
    assert!(close(px(&a, v2(250.0, 200.0)), GREEN, 3));

    let at = v2(250.0, 200.0);
    scroll(&mut s.e, 200.0, at);
    assert_eq!(scroll_y(&s.e), 0.0, "the panel scrolled, not the page");
    let b = view.draw(&mut s.e);
    assert!(
        close(px(&b, v2(250.0, 250.0)), YELLOW, 3),
        "its content moved"
    );
    assert!(
        close(px(&b, v2(250.0, 95.0)), WHITE, 3),
        "and stays clipped to it"
    );

    // The panel reaches its end at 300, then the page takes over.
    scroll(&mut s.e, 100.0, at);
    scroll(&mut s.e, 50.0, at);
    assert_eq!(scroll_y(&s.e), 50.0);
    assert_eq!(s.e.doc.get(panel).unwrap().children.len(), 2);
    let source =
        s.e.doc
            .get(s.e.doc.get(panel).unwrap().children[0])
            .unwrap();
    assert_eq!(source.rect.y, 0.0, "the document didn't scroll");
}

/// Hovering recolors exactly the layers that changed.
#[test]
fn hovering_redraws_only_the_layers_that_changed() {
    let Some(g) = gpu() else { return };
    let mut s = site();
    let button = s.button;
    s.e.edit("hover", |tx| {
        tx.node_mut(button).unwrap().edit_proto(|p| {
            p.hover = Some(StateStyle {
                fill: Some(BLUE),
                ..StateStyle::default()
            });
        });
    });
    start(&mut s);
    let mut view = Screenful::new(g);
    let a = view.draw(&mut s.e);
    assert!(close(px(&a, v2(150.0, 120.0)), RED, 3));
    hover(&mut s.e, v2(150.0, 120.0), 10.0);
    let b = view.draw(&mut s.e);
    assert!(close(px(&b, v2(150.0, 120.0)), BLUE, 3), "hover shows");
    assert_eq!(view.cache.last_misses(), 1, "only the button was redone");
    hover(&mut s.e, v2(800.0, 500.0), 20.0);
    let c = view.draw(&mut s.e);
    assert!(close(px(&c, v2(150.0, 120.0)), RED, 3), "and goes away");
}

fn animated_site(kind: TransitionKind) -> Site {
    let mut s = site();
    let target = Target {
        page: Some(s.about_page.clone()),
        node: s.about,
    };
    set_interactions(
        &mut s.e,
        s.button,
        vec![Interaction {
            trigger: Trigger::Click,
            action: ProtoAction::Navigate {
                target,
                transition: Transition {
                    kind,
                    dir: SlideDir::Left,
                    ms: 1000.0,
                    easing: sketch_rs::model::Easing::Linear,
                },
            },
        }],
    );
    s
}

#[test]
fn dissolve_fades_the_new_screen_in_over_the_old_one() {
    let Some(g) = gpu() else { return };
    let mut s = animated_site(TransitionKind::Dissolve);
    start(&mut s);
    click(&mut s.e, v2(150.0, 120.0), 0.0);
    let mut view = Screenful::new(g);
    s.e.tick(500.0);
    let mid = view.draw(&mut s.e);
    // Halfway from white (home) to blue (about).
    let p = px(&mid, v2(800.0, 500.0));
    let (mr, mg, mb) = (
        (WHITE.r as i32 + BLUE.r as i32) / 2,
        (WHITE.g as i32 + BLUE.g as i32) / 2,
        (WHITE.b as i32 + BLUE.b as i32) / 2,
    );
    assert!(
        (p[0] as i32 - mr).abs() < 6
            && (p[1] as i32 - mg).abs() < 6
            && (p[2] as i32 - mb).abs() < 6,
        "{p:?}"
    );
    s.e.tick(1100.0);
    let end = view.draw(&mut s.e);
    assert!(close(px(&end, v2(800.0, 500.0)), BLUE, 3));
}

#[test]
fn push_slides_both_screens_and_back_plays_it_in_reverse() {
    let Some(g) = gpu() else { return };
    let mut s = animated_site(TransitionKind::Push);
    start(&mut s);
    click(&mut s.e, v2(150.0, 120.0), 0.0);
    let mut view = Screenful::new(g);
    s.e.tick(500.0);
    let uploads = view.r.uploads;
    let mid = view.draw(&mut s.e);
    assert!(
        close(px(&mid, v2(250.0, 500.0)), WHITE, 3),
        "old screen, half gone"
    );
    assert!(
        close(px(&mid, v2(750.0, 500.0)), BLUE, 3),
        "new screen, half in"
    );
    s.e.tick(750.0);
    let later = view.draw(&mut s.e);
    // The new screen's edge has reached x=250, where it was at x=500 before.
    assert!(
        close(px(&later, v2(300.0, 500.0)), BLUE, 3),
        "it kept sliding"
    );
    assert_eq!(view.r.uploads, uploads + 1, "only the first frame uploaded");

    s.e.tick(1100.0);
    view.draw(&mut s.e);
    let t = 2000.0;
    s.e.tick(t);
    press(&mut s.e, Key::Left);
    s.e.tick(t + 500.0);
    let back = view.draw(&mut s.e);
    // The old screen comes back from the left this time.
    assert!(
        close(px(&back, v2(250.0, 500.0)), WHITE, 3),
        "{:?}",
        px(&back, v2(250.0, 500.0))
    );
    assert!(close(px(&back, v2(750.0, 500.0)), BLUE, 3));
    assert_eq!(showing(&s.e), s.home);
}

#[test]
fn an_overlay_darkens_the_screen_behind_it() {
    let Some(g) = gpu() else { return };
    let (mut s, _) = overlay_site(false, OverlayPlacement::Center);
    start(&mut s);
    let mut view = Screenful::new(g);
    let a = view.draw(&mut s.e);
    assert!(close(px(&a, v2(800.0, 500.0)), WHITE, 3));
    click(&mut s.e, v2(150.0, 120.0), 10.0);
    let b = view.draw(&mut s.e);
    assert!(close(px(&b, v2(500.0, 300.0)), YELLOW, 3), "the overlay");
    let dim = px(&b, v2(800.0, 500.0));
    assert!(
        dim[0] < 160 && dim[0] > 110,
        "darkened by the backdrop: {dim:?}"
    );
    assert_eq!(dim[0], dim[1]);
}

#[test]
fn the_frame_fills_the_window_below_its_end_when_it_fills_the_width() {
    let Some(g) = gpu() else { return };
    let mut s = site();
    let short =
        s.e.add_node(frame_node(Rect::new(0.0, 0.0, 1000.0, 200.0), RED), None);
    assert!(s.e.start_preview(Some((0, short))));
    let mut view = Screenful::new(g);
    let a = view.draw(&mut s.e);
    assert!(
        close(px(&a, v2(500.0, 500.0)), RED, 3),
        "like a body background"
    );

    let phone =
        s.e.add_node(frame_node(Rect::new(0.0, 500.0, 400.0, 200.0), RED), None);
    s.e.stop_preview();
    assert!(s.e.start_preview(Some((0, phone))));
    let b = view.draw(&mut s.e);
    assert!(
        close(px(&b, v2(100.0, 100.0)), render::STAGE, 3),
        "narrow: the stage"
    );
}

#[test]
fn the_full_window_shows_the_bar_the_panel_and_links_on_the_canvas() {
    let Some(g) = gpu() else { return };
    let mut s = site();
    let (home, button) = (s.home, s.button);
    let phone = s.e.add_node(
        frame_node(Rect::new(1500.0, 50.0, 390.0, 800.0), YELLOW),
        None,
    );
    let extra = s.e.add_node(
        box_node(Rect::new(400.0, 300.0, 200.0, 50.0), GREEN),
        Some(home),
    );
    set_interactions(&mut s.e, extra, vec![click_to(Target::same_page(phone))]);
    s.e.edit("hover", |tx| {
        tx.node_mut(button).unwrap().edit_proto(|p| {
            p.hover = Some(StateStyle {
                fill: Some(BLUE),
                ..StateStyle::default()
            });
        });
    });
    let mut st = UiState::default();

    s.e.selection = vec![button];
    s.e.proto_mode = true;
    let editing = frame::capture(g, &mut s.e, &mut st, [1440.0, 900.0], 2.0, |e| {
        e.zoom_to_fit()
    });
    editing.save(artifact("panel.png")).unwrap();

    s.e.selection.clear();
    let id = s.home;
    assert!(s.e.start_preview(Some((0, id))));
    let playing = frame::capture(g, &mut s.e, &mut st, [1440.0, 900.0], 2.0, |_| {});
    playing.save(artifact("preview.png")).unwrap();
    assert_ne!(editing.as_raw(), playing.as_raw());
    // The window's width is more than the frame's, so it's centered at 100%.
    let cam = s.e.preview.as_ref().unwrap().views()[0].zoom;
    assert_eq!(cam, 1.0);
    assert!(
        s.e.viewport.x >= 1440.0 - 1.0,
        "no panels: {:?}",
        s.e.viewport
    );

    s.e.with_preview(|p, _| p.show_hotspots = true);
    let hints = frame::capture(g, &mut s.e, &mut st, [1440.0, 900.0], 2.0, |_| {});
    hints.save(artifact("hotspots.png")).unwrap();
    assert_ne!(playing.as_raw(), hints.as_raw(), "hotspots show");
}

#[test]
fn scrolling_a_big_page_rebuilds_no_layers() {
    let Some(g) = gpu() else { return };
    let mut s = site();
    let home = s.home;
    for i in 0..400 {
        let y = i as f64 * 5.0;
        s.e.add_node(box_node(Rect::new(0.0, y, 900.0, 4.0), RED), Some(home));
    }
    start(&mut s);
    let mut view = Screenful::new(g);
    view.draw(&mut s.e);
    assert!(view.cache.last_misses() >= 400);
    let uploads = view.r.uploads;
    for _ in 0..10 {
        scroll(&mut s.e, 40.0, v2(950.0, 300.0));
        s.e.tick(s.e.now_ms + 16.0);
        view.draw(&mut s.e);
    }
    assert_eq!(scroll_y(&s.e), 400.0);
    assert_eq!(view.r.uploads, uploads);
    let r = NodeRef {
        page: s.e.pages[0].id.clone(),
        node: home,
    };
    let on_screen = s.e.preview.as_ref().unwrap().node_screen_rect(&r).unwrap();
    assert_eq!(
        on_screen.y, -400.0,
        "layers know where they are after scrolling"
    );
}
