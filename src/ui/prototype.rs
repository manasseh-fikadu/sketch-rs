//! The Prototype panel: what a layer links to, how it looks when hovered or pressed, and how it
//! behaves when the screen scrolls. Everything here edits `Node::proto`, as one undo step per
//! change and as overrides inside instances.

use super::inspector::{apply, color_button, combo, num_field, row, separator};
use super::title_text;
use crate::editor::Editor;
use crate::editor::shortcuts::{self, Action};
use crate::geom::v2;
use crate::model::{
    Color, Easing, Interaction, Node, NodeId, Overflow, OverlayPlacement, ProtoAction, Prototype,
    ScrollPosition, SlideDir, Target, Transition, TransitionKind, Trigger,
};
use crate::theme::{self, Icon};
use egui::{RichText, Ui};

const FIELD_W: f32 = 150.0;

fn note(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(theme::SECONDARY)
            .color(theme::pal(ui).secondary),
    );
}

pub fn preview_button(ui: &mut Ui, ed: &mut Editor) {
    let r = ui
        .add(
            egui::Button::new(RichText::new("Preview").font(theme::medium(theme::BODY)))
                .min_size(egui::vec2(ui.available_width(), 26.0)),
        )
        .on_hover_text(format!(
            "Play this frame like a website ({})",
            shortcuts::chord_text(Action::TogglePreview)
        ));
    if r.clicked() {
        ed.run(Action::TogglePreview);
    }
}

pub fn panel(ui: &mut Ui, ed: &mut Editor) {
    preview_button(ui, ed);
    ui.add_space(4.0);
    let ids = ed.top_selection();
    let Some(first) = ids.first().and_then(|id| ed.doc.get(*id)).cloned() else {
        note(
            ui,
            "Select a layer to give it links, a hover state and scroll behavior.",
        );
        return;
    };
    let title = if ids.len() > 1 {
        format!("{} layers", ids.len())
    } else {
        first.name.clone()
    };
    ui.label(title_text(title));
    separator(ui);
    interactions(ui, ed, &ids, &first);
    separator(ui);
    state_section(ui, ed, &ids, &first, State::Hover);
    ui.add_space(4.0);
    state_section(ui, ed, &ids, &first, State::Pressed);
    separator(ui);
    scrolling(ui, ed, &ids, &first);
}

/// Apply one change to the selection's prototype settings. Dragging a number stays one undo step;
/// anything else is its own.
fn change(ui: &Ui, ed: &mut Editor, ids: &[NodeId], f: impl Fn(&mut Prototype)) {
    apply(ed, "prototype", ids, |n| n.edit_proto(&f));
    if ui.ctx().dragged_id().is_none() {
        ed.history.seal();
    }
}

// ---------------------------------------------------------------------------------- interactions

fn interactions(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    let mut list: Vec<Interaction> = first.interactions().to_vec();
    let before = list.clone();
    let page = ed.active_page_id().to_string();
    // A new link starts on the frame it's in, until a target is picked.
    let home = ed
        .doc
        .ancestors(first.id)
        .last()
        .copied()
        .unwrap_or(first.id);

    let mut add = false;
    ui.horizontal(|ui| {
        ui.label(title_text("Interactions"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            add = theme::icon_button(ui, Icon::Plus, 20.0, false)
                .on_hover_text("Add an interaction")
                .clicked();
        });
    });
    if list.is_empty() && !add {
        note(
            ui,
            "Nothing happens yet. Add an interaction, or drag the knob beside the layer onto a frame.",
        );
    }
    let mut remove = None;
    for (i, it) in list.iter_mut().enumerate() {
        ui.push_id(i, |ui| {
            egui::Frame::group(ui.style())
                .inner_margin(6.0)
                .show(ui, |ui| {
                    if interaction(ui, ed, it, &page, home) {
                        remove = Some(i);
                    }
                });
        });
    }
    if let Some(i) = remove {
        list.remove(i);
    }
    if add {
        list.push(Interaction {
            trigger: Trigger::Click,
            action: ProtoAction::Navigate {
                target: Target::same_page(home),
                transition: Transition::default(),
            },
        });
    }
    if list != before {
        change(ui, ed, ids, |p| p.interactions = list.clone());
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Navigate,
    Back,
    Overlay,
    CloseOverlay,
    Url,
}

fn kind_of(a: &ProtoAction) -> Kind {
    match a {
        ProtoAction::Navigate { .. } => Kind::Navigate,
        ProtoAction::Back => Kind::Back,
        ProtoAction::OpenOverlay { .. } => Kind::Overlay,
        ProtoAction::CloseOverlay => Kind::CloseOverlay,
        ProtoAction::OpenUrl { .. } => Kind::Url,
    }
}

/// `a` turned into another kind of action, keeping its target and transition.
fn convert(a: &ProtoAction, kind: Kind, home: NodeId) -> ProtoAction {
    let target = a
        .target()
        .cloned()
        .unwrap_or_else(|| Target::same_page(home));
    let transition = match a {
        ProtoAction::Navigate { transition, .. } | ProtoAction::OpenOverlay { transition, .. } => {
            *transition
        }
        _ => Transition::default(),
    };
    match kind {
        Kind::Navigate => ProtoAction::Navigate { target, transition },
        Kind::Back => ProtoAction::Back,
        Kind::Overlay => ProtoAction::OpenOverlay {
            target,
            placement: OverlayPlacement::Center,
            dim: true,
            close_outside: true,
            transition,
        },
        Kind::CloseOverlay => ProtoAction::CloseOverlay,
        Kind::Url => ProtoAction::OpenUrl {
            url: "https://".into(),
        },
    }
}

/// One interaction's controls. Returns whether its remove button was clicked.
fn interaction(
    ui: &mut Ui,
    ed: &mut Editor,
    it: &mut Interaction,
    page: &str,
    home: NodeId,
) -> bool {
    let mut remove = false;
    #[derive(Clone, Copy, PartialEq)]
    enum T {
        Click,
        Hover,
        Delay,
    }
    let cur = match it.trigger {
        Trigger::Click => T::Click,
        Trigger::Hover => T::Hover,
        Trigger::Delay { .. } => T::Delay,
    };
    row(ui, "When", |ui| {
        let options = [
            (T::Click, "Clicked"),
            (T::Hover, "Hovered"),
            (T::Delay, "After a delay"),
        ];
        if let Some(t) = combo(ui, "trigger", cur, &options, FIELD_W - 28.0) {
            it.trigger = match t {
                T::Click => Trigger::Click,
                T::Hover => Trigger::Hover,
                T::Delay => Trigger::Delay { ms: 2000.0 },
            };
        }
        remove = theme::icon_button(ui, Icon::Trash, 20.0, false)
            .on_hover_text("Remove")
            .clicked();
    });
    if let Trigger::Delay { ms } = &mut it.trigger {
        row(ui, "Wait", |ui| {
            if let Some(v) = num_field(ui, ed, "", *ms, 10.0, 0.0..=600_000.0, 80.0) {
                *ms = v;
            }
            ui.label("ms");
        });
    }
    row(ui, "Do", |ui| {
        let options = [
            (Kind::Navigate, "Go to"),
            (Kind::Back, "Go back"),
            (Kind::Overlay, "Open overlay"),
            (Kind::CloseOverlay, "Close overlay"),
            (Kind::Url, "Open link"),
        ];
        if let Some(k) = combo(ui, "action", kind_of(&it.action), &options, FIELD_W) {
            it.action = convert(&it.action, k, home);
        }
    });
    match &mut it.action {
        ProtoAction::Navigate { target, transition } => {
            target_picker(ui, ed, target, page);
            transition_controls(ui, ed, transition);
        }
        ProtoAction::OpenOverlay {
            target,
            placement,
            dim,
            close_outside,
            transition,
        } => {
            target_picker(ui, ed, target, page);
            row(ui, "Place", |ui| {
                let options = [
                    (OverlayPlacement::Center, "Center"),
                    (OverlayPlacement::Below, "Below this layer"),
                    (OverlayPlacement::TopLeft, "Top left"),
                    (OverlayPlacement::TopCenter, "Top center"),
                    (OverlayPlacement::TopRight, "Top right"),
                    (OverlayPlacement::BottomLeft, "Bottom left"),
                    (OverlayPlacement::BottomCenter, "Bottom center"),
                    (OverlayPlacement::BottomRight, "Bottom right"),
                ];
                if let Some(v) = combo(ui, "placement", *placement, &options, FIELD_W) {
                    *placement = v;
                }
            });
            if it.trigger == Trigger::Hover {
                note(
                    ui,
                    "Closes when the pointer leaves this layer and the overlay.",
                );
            } else {
                ui.checkbox(dim, "Darken what's behind");
                ui.checkbox(close_outside, "Close when clicking outside");
            }
            transition_controls(ui, ed, transition);
        }
        ProtoAction::OpenUrl { url } => {
            ui.add(
                egui::TextEdit::singleline(url)
                    .hint_text("https://example.com")
                    .desired_width(f32::INFINITY),
            );
            if !crate::preview::is_safe_url(url) {
                note(ui, "Use an http, https, mailto or tel link.");
            }
        }
        ProtoAction::Back | ProtoAction::CloseOverlay => {}
    }
    remove
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max - 1).collect();
        format!("{head}…")
    }
}

/// The screen a link opens, from every page of the project, then a section of it.
fn target_picker(ui: &mut Ui, ed: &Editor, target: &mut Target, page: &str) {
    let resolved = ed.resolve_target(page, target);
    let label = match &resolved {
        Some(r) => {
            let name = ed
                .page_doc(r.page_index)
                .get(r.frame)
                .map_or("", |n| n.name.as_str());
            short(name, 28)
        }
        None => "Target missing".to_string(),
    };
    let secondary = theme::pal(ui).secondary;
    row(ui, "Screen", |ui| {
        egui::ComboBox::from_id_salt("target")
            .selected_text(label)
            .width(FIELD_W)
            .height(420.0)
            .show_ui(ui, |ui| {
                for (i, slot) in ed.pages.iter().enumerate() {
                    let doc = ed.page_doc(i);
                    let frames: Vec<&Node> = doc
                        .root
                        .iter()
                        .filter_map(|id| doc.get(*id))
                        .filter(|n| n.is_frame())
                        .collect();
                    if frames.is_empty() {
                        continue;
                    }
                    ui.label(
                        RichText::new(&slot.name)
                            .size(theme::SECONDARY)
                            .color(secondary),
                    );
                    for f in frames {
                        let on = resolved
                            .as_ref()
                            .is_some_and(|r| r.page_index == i && r.frame == f.id);
                        if ui.selectable_label(on, &f.name).clicked() {
                            *target = Target {
                                page: (slot.id != page).then(|| slot.id.clone()),
                                node: f.id,
                            };
                        }
                    }
                }
            });
    });
    // Picking a screen above replaced `target`, so look it up again.
    let Some(r) = ed.resolve_target(page, target) else {
        return;
    };
    let doc = ed.page_doc(r.page_index);
    let mut sections = vec![(r.frame, "Top of page".to_string())];
    let named = |id: &NodeId| {
        doc.get(*id)
            .filter(|n| n.is_frame() && n.visible)
            .map(|n| (n.id, n.name.clone(), n.children.clone()))
    };
    for (id, name, kids) in doc
        .get(r.frame)
        .iter()
        .flat_map(|f| f.children.iter())
        .filter_map(named)
    {
        sections.push((id, name));
        for (kid, kid_name, _) in kids.iter().filter_map(named) {
            sections.push((kid, format!("   {kid_name}")));
        }
    }
    if sections.len() < 2 {
        return;
    }
    let options: Vec<(NodeId, &str)> = sections.iter().map(|(i, n)| (*i, n.as_str())).collect();
    row(ui, "Scroll to", |ui| {
        if let Some(v) = combo(ui, "section", r.node, &options, FIELD_W) {
            target.node = v;
        }
    });
}

fn transition_controls(ui: &mut Ui, ed: &mut Editor, t: &mut Transition) {
    row(ui, "Animate", |ui| {
        let options = [
            (TransitionKind::Instant, "None"),
            (TransitionKind::Dissolve, "Dissolve"),
            (TransitionKind::MoveIn, "Slide in"),
            (TransitionKind::Push, "Push"),
        ];
        if let Some(v) = combo(ui, "transition", t.kind, &options, FIELD_W) {
            t.kind = v;
        }
    });
    if t.kind == TransitionKind::Instant {
        return;
    }
    if matches!(t.kind, TransitionKind::MoveIn | TransitionKind::Push) {
        row(ui, "Direction", |ui| {
            let options = [
                (SlideDir::Left, "Left"),
                (SlideDir::Right, "Right"),
                (SlideDir::Up, "Up"),
                (SlideDir::Down, "Down"),
            ];
            if let Some(v) = combo(ui, "direction", t.dir, &options, FIELD_W) {
                t.dir = v;
            }
        });
    }
    row(ui, "Easing", |ui| {
        let options = [
            (Easing::EaseOut, "Ease out"),
            (Easing::EaseIn, "Ease in"),
            (Easing::EaseInOut, "Ease in and out"),
            (Easing::Linear, "Linear"),
        ];
        if let Some(v) = combo(ui, "easing", t.easing, &options, FIELD_W) {
            t.easing = v;
        }
    });
    row(ui, "Duration", |ui| {
        if let Some(v) = num_field(ui, ed, "", t.ms, 5.0, 1.0..=5000.0, 80.0) {
            t.ms = v;
        }
        ui.label("ms");
    });
}

// ---------------------------------------------------------------------------------- states

#[derive(Clone, Copy)]
enum State {
    Hover,
    Pressed,
}

fn current_color(first: &Node) -> Color {
    first.style.fill().unwrap_or(Color::rgb(0x2f, 0x6f, 0xf3))
}

fn opt_color(ui: &mut Ui, ed: &mut Editor, label: &str, v: &mut Option<Color>, default: Color) {
    row(ui, label, |ui| {
        let mut on = v.is_some();
        if ui.checkbox(&mut on, "").changed() {
            *v = on.then_some(default);
        }
        if let Some(c) = *v
            && let Some(n) = color_button(ui, ed, c)
        {
            *v = Some(n);
        }
    });
}

fn state_section(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node, which: State) {
    let proto = first.proto.as_deref();
    let (title, cur) = match which {
        State::Hover => ("Hover state", proto.and_then(|p| p.hover)),
        State::Pressed => ("Pressed state", proto.and_then(|p| p.pressed)),
    };
    let mut enabled = cur.is_some();
    let mut st = cur.unwrap_or_default();
    ui.horizontal(|ui| {
        ui.checkbox(&mut enabled, title_text(title));
    });
    if enabled {
        ui.push_id(title, |ui| {
            opt_color(ui, ed, "Fill", &mut st.fill, current_color(first));
            opt_color(ui, ed, "Text", &mut st.text, Color::WHITE);
            opt_color(
                ui,
                ed,
                "Border",
                &mut st.stroke,
                first.style.stroke.unwrap_or(Color::BLACK),
            );
            row(ui, "Opacity", |ui| {
                let mut on = st.opacity.is_some();
                if ui.checkbox(&mut on, "").changed() {
                    st.opacity = on.then_some(0.8);
                }
                if let Some(o) = st.opacity
                    && let Some(n) = num_field(ui, ed, "", o * 100.0, 1.0, 0.0..=100.0, 64.0)
                {
                    st.opacity = Some(n / 100.0);
                }
            });
            row(ui, "Move up", |ui| {
                let mut on = st.offset.is_some();
                if ui.checkbox(&mut on, "").changed() {
                    st.offset = on.then_some(v2(0.0, -2.0));
                }
                if let Some(d) = st.offset
                    && let Some(n) = num_field(ui, ed, "", -d.y, 0.5, -200.0..=200.0, 64.0)
                {
                    st.offset = Some(v2(d.x, -n));
                }
            });
        });
    }
    // Checked with nothing chosen yet still counts, so the fields stay open while choosing.
    let next = enabled.then_some(st);
    if next != cur {
        change(ui, ed, ids, |p: &mut Prototype| match which {
            State::Hover => p.hover = next,
            State::Pressed => p.pressed = next,
        });
    }
}

// ---------------------------------------------------------------------------------- scrolling

fn scrolling(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    ui.label(title_text("Scrolling"));
    if first.parent.is_none() {
        note(
            ui,
            "A top-level frame scrolls by itself when it's taller than the window.",
        );
        return;
    }
    let proto = first.proto.as_deref();
    let position = proto.map_or(ScrollPosition::Scrolls, |p| p.position);
    row(ui, "Position", |ui| {
        let options = [
            (ScrollPosition::Scrolls, "Scrolls with page"),
            (ScrollPosition::Sticky, "Sticks at the top"),
            (ScrollPosition::Fixed, "Fixed in place"),
        ];
        if let Some(v) = combo(ui, "scroll-position", position, &options, FIELD_W) {
            change(ui, ed, ids, |p| p.position = v);
        }
    });
    if !first.is_frame() {
        return;
    }
    let overflow = proto.map_or(Overflow::None, |p| p.overflow);
    row(ui, "Overflow", |ui| {
        let options = [
            (Overflow::None, "Doesn't scroll"),
            (Overflow::Vertical, "Scrolls vertically"),
            (Overflow::Horizontal, "Scrolls sideways"),
            (Overflow::Both, "Scrolls both ways"),
        ];
        if let Some(v) = combo(ui, "scroll-overflow", overflow, &options, FIELD_W) {
            apply(ed, "prototype", ids, |n| {
                n.edit_proto(|p| p.overflow = v);
                // Content that scrolls has to be cut off at the frame's edge.
                if v != Overflow::None
                    && let crate::model::NodeKind::Frame { clip } = &mut n.kind
                {
                    *clip = true;
                }
            });
            ed.history.seal();
        }
    });
}
