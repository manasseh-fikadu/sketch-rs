//! The Design inspector: macOS-style grouped sections for the selection's geometry, layout,
//! constraints, text, fills, stroke and layer properties. Edits are coalesced per field so a
//! drag on a number is one undo step, and edits inside instances become overrides.

use super::{UiState, c32, from32, section, title_text};
use crate::editor::vector_ops::BoolOp;
use crate::editor::{AppRequest, Editor};
use crate::geom::v2;
use crate::model::{
    Align, AutoLayout, BlendMode, Cap, Color, Constraint, Direction, ImageMode, Join, Node, NodeId,
    NodeKind, Paint, Sizing, Stop, StrokeAlign, TextAlign,
};
use crate::theme;
use egui::{RichText, Ui};

const LABEL_W: f32 = 64.0;

/// Apply a property edit as a coalesced transaction keyed by field + selection.
pub fn apply(ed: &mut Editor, field: &str, ids: &[NodeId], f: impl Fn(&mut Node)) {
    let key = format!("{field}:{ids:?}");
    ed.begin_coalescing(&format!("Change {field}"), Some(&key));
    {
        let mut tx = ed.tx();
        for id in ids {
            if let Some(n) = tx.node_mut(*id) {
                f(n);
            }
        }
    }
    ed.mark_overrides(ids, field);
    for id in ids {
        ed.fit_text(*id);
    }
    ed.commit();
}

fn seal_on_release(ed: &mut Editor, r: &egui::Response) {
    if r.drag_stopped() || r.lost_focus() {
        ed.history.seal();
    }
}

/// A row: secondary label on the left, controls on the right.
fn row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(LABEL_W, 20.0), egui::Sense::hover());
        ui.painter().text(
            r.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(theme::SECONDARY + 1.0),
            theme::pal(ui).secondary,
        );
        add(ui)
    })
    .inner
}

/// A compact number field with a leading letter (like Figma's "X 120").
fn num_field(
    ui: &mut Ui,
    ed: &mut Editor,
    prefix: &str,
    value: f64,
    speed: f64,
    range: std::ops::RangeInclusive<f64>,
    width: f32,
) -> Option<f64> {
    let mut v = value;
    let r = ui.add_sized(
        egui::vec2(width, 20.0),
        egui::DragValue::new(&mut v)
            .speed(speed)
            .range(range)
            .max_decimals(2)
            .prefix(prefix),
    );
    seal_on_release(ed, &r);
    (r.changed() && v != value).then_some(v)
}

fn color_button(ui: &mut Ui, ed: &mut Editor, c: Color) -> Option<Color> {
    let mut c2 = c32(c);
    let r = ui.color_edit_button_srgba(&mut c2);
    seal_on_release(ed, &r);
    (r.changed() && from32(c2) != c).then(|| from32(c2))
}

fn combo<T: PartialEq + Copy>(
    ui: &mut Ui,
    id: &str,
    current: T,
    options: &[(T, &str)],
    width: f32,
) -> Option<T> {
    let mut v = current;
    let label = options
        .iter()
        .find(|(o, _)| *o == current)
        .map_or("—", |(_, l)| *l);
    egui::ComboBox::from_id_salt(id)
        .selected_text(label)
        .width(width)
        .show_ui(ui, |ui| {
            for (o, l) in options {
                ui.selectable_value(&mut v, *o, *l);
            }
        });
    (v != current).then_some(v)
}

fn separator(ui: &mut Ui) {
    ui.add_space(6.0);
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(
        r.x_range(),
        r.top(),
        egui::Stroke::new(1.0, theme::pal(ui).separator),
    );
    ui.add_space(8.0);
}

fn icon_btn(ui: &mut Ui, icon: theme::Icon, on: bool, tip: &str) -> bool {
    theme::icon_button(ui, icon, 24.0, on)
        .on_hover_text(tip)
        .clicked()
}

fn icon_toggle(ui: &mut Ui, label: &str, on: bool, tip: &str) -> bool {
    ui.add(
        egui::Button::selectable(on, RichText::new(label).font(theme::semibold(12.0)))
            .min_size(egui::vec2(24.0, 20.0)),
    )
    .on_hover_text(tip)
    .clicked()
}

pub fn panel(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            section(ui, "Design");
            ui.add_space(2.0);
            let ids = ed.top_selection();
            if ids.is_empty() {
                nothing_selected(ui, ed);
                separator(ui);
                export_section(ui, ed, st);
                return;
            }
            let first = ed.doc.get(ids[0]).cloned().unwrap();
            header(ui, ed, &ids, &first);
            separator(ui);
            align_row(ui, ed, &ids);
            separator(ui);
            geometry(ui, ed, &ids, &first);
            child_layout(ui, ed, &ids, &first);
            if first.is_frame() && ids.len() == 1 {
                auto_layout(ui, ed, &ids, &first);
            }
            if matches!(first.kind, NodeKind::Text { .. }) {
                separator(ui);
                text_section(ui, ed, &ids, &first);
            }
            if crate::render::draw::is_area(&first)
                && !matches!(first.kind, NodeKind::Stroke { .. })
            {
                separator(ui);
                fills(ui, ed, &ids, &first);
            }
            if !matches!(
                first.kind,
                NodeKind::Text { .. } | NodeKind::Stroke { .. } | NodeKind::Group
            ) {
                separator(ui);
                stroke(ui, ed, &ids, &first);
            }
            if matches!(first.kind, NodeKind::Connector { .. }) {
                separator(ui);
                arrow_section(ui, ed, &ids, &first);
            }
            if matches!(first.kind, NodeKind::Sticky { .. }) {
                separator(ui);
                sticky_section(ui, ed, &ids, &first);
            }
            separator(ui);
            layer(ui, ed, &ids, &first);
            if !matches!(
                first.kind,
                NodeKind::Text { .. }
                    | NodeKind::Stroke { .. }
                    | NodeKind::Group
                    | NodeKind::Sticky { .. }
            ) {
                separator(ui);
                sketch_section(ui, ed, &ids, &first);
            }
            separator(ui);
            export_section(ui, ed, st);
        });
}

/// FigJam/Excalidraw-style sticky colours.
pub const STICKY_COLORS: [Color; 6] = [
    Color::rgb(0xff, 0xdf, 0x6b),
    Color::rgb(0xff, 0xb8, 0xa8),
    Color::rgb(0xb5, 0xe6, 0xa2),
    Color::rgb(0xa8, 0xd8, 0xff),
    Color::rgb(0xd7, 0xc2, 0xff),
    Color::rgb(0xe8, 0xe8, 0xec),
];

fn swatches(ui: &mut Ui, current: Color) -> Option<Color> {
    let mut out = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for c in STICKY_COLORS {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
            ui.painter().rect_filled(r, 5.0, c32(c));
            if c == current {
                ui.painter().rect_stroke(
                    r,
                    5.0,
                    egui::Stroke::new(2.0, theme::accent()),
                    egui::StrokeKind::Outside,
                );
            }
            if resp.clicked() {
                out = Some(c);
            }
        }
    });
    out
}

fn sticky_section(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    ui.label(title_text("Sticky note"));
    let cur = first.style.fill().unwrap_or(STICKY_COLORS[0]);
    if let Some(c) = swatches(ui, cur) {
        apply(ed, "fill", ids, |n| n.style.set_fill(Some(c)));
        ed.sticky_color = c;
    }
    if let NodeKind::Sticky { font_size, .. } = first.kind {
        row(ui, "Font size", |ui| {
            if let Some(v) = num_field(ui, ed, "", font_size, 0.5, 8.0..=200.0, 70.0) {
                apply(ed, "font size", ids, |n| {
                    if let NodeKind::Sticky { font_size, .. } = &mut n.kind {
                        *font_size = v;
                    }
                });
            }
        });
    }
}

fn arrow_section(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    use crate::model::{Head, Route};
    let NodeKind::Connector {
        route,
        start_head,
        end_head,
        start,
        end,
        ..
    } = &first.kind
    else {
        return;
    };
    ui.label(title_text("Arrow"));
    let routes = [
        (Route::Straight, "Straight"),
        (Route::Curved, "Curved"),
        (Route::Elbow, "Elbow"),
    ];
    row(ui, "Type", |ui| {
        if let Some(v) = combo(ui, "route", *route, &routes, 96.0) {
            apply(ed, "route", ids, |n| {
                if let NodeKind::Connector { route, points, .. } = &mut n.kind {
                    *route = v;
                    if v != Route::Elbow && points.len() > 2 && v == Route::Straight {
                        let (a, b) = (points[0], *points.last().unwrap());
                        *points = vec![a, b];
                    }
                }
            });
            ed.arrow_route = v;
            // Re-route through the binding logic.
            ed.begin("Change route");
            {
                let mut tx = ed.tx();
                for id in ids {
                    crate::editor::whiteboard::update_connector(&mut tx, *id);
                }
            }
            ed.commit();
        }
    });
    let heads: Vec<(Head, &str)> = Head::ALL.iter().map(|h| (*h, h.label())).collect();
    row(ui, "Heads", |ui| {
        if let Some(v) = combo(ui, "h-start", *start_head, &heads, 72.0) {
            apply(ed, "heads", ids, |n| {
                if let NodeKind::Connector { start_head, .. } = &mut n.kind {
                    *start_head = v;
                }
            });
        }
        if let Some(v) = combo(ui, "h-end", *end_head, &heads, 72.0) {
            apply(ed, "heads", ids, |n| {
                if let NodeKind::Connector { end_head, .. } = &mut n.kind {
                    *end_head = v;
                }
            });
        }
    });
    let bound = |b: &Option<crate::model::Binding>| {
        b.and_then(|b| ed.doc.get(b.node).map(|n| n.name.clone()))
            .unwrap_or_else(|| "—".into())
    };
    ui.label(
        RichText::new(format!("Connects {} → {}", bound(start), bound(end)))
            .size(theme::SECONDARY)
            .color(theme::pal(ui).secondary),
    );
}

fn sketch_section(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    use crate::model::rough::{FillStyle, Rough};
    ui.horizontal(|ui| {
        ui.label(title_text("Hand-drawn"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut on = first.style.rough.is_some();
            if ui
                .checkbox(&mut on, "")
                .on_hover_text("Sketchy rough.js-style lines (⌘⌥H)")
                .changed()
            {
                ed.toggle_sketch();
            }
        });
    });
    let Some(r) = first.style.rough else { return };
    let levels = [(0u8, "Architect"), (1, "Artist"), (2, "Cartoonist")];
    let cur = r.roughness.round().clamp(0.0, 2.0) as u8;
    row(ui, "Sloppiness", |ui| {
        if let Some(v) = combo(ui, "rough", cur, &levels, 96.0) {
            apply(ed, "rough", ids, |n| {
                if let Some(r) = &mut n.style.rough {
                    r.roughness = v as f64;
                }
            });
        }
    });
    let fills: Vec<(FillStyle, &str)> = FillStyle::ALL.iter().map(|f| (*f, f.label())).collect();
    row(ui, "Fill style", |ui| {
        if let Some(v) = combo(ui, "fillstyle", r.fill_style, &fills, 96.0) {
            apply(ed, "rough", ids, |n| {
                if let Some(r) = &mut n.style.rough {
                    r.fill_style = v;
                }
            });
        }
    });
    if ui
        .small_button("New wobble")
        .on_hover_text("Re-roll the random seed")
        .clicked()
    {
        apply(ed, "rough", ids, |n| {
            if let Some(r) = &mut n.style.rough {
                *r = Rough {
                    seed: r.seed.wrapping_mul(1103515245).wrapping_add(12345) | 1,
                    ..*r
                };
            }
        });
    }
}

/// Export settings and button, like Figma's Export section.
fn export_section(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    use crate::io::export::{Format, Scope};
    ui.label(title_text("Export"));
    let spec = &mut st.export;
    ui.horizontal(|ui| {
        let fmts: Vec<(Format, &str)> = Format::ALL
            .iter()
            .map(|f| {
                (
                    *f,
                    match f {
                        Format::Png => "PNG",
                        Format::Svg => "SVG",
                        Format::Pdf => "PDF",
                    },
                )
            })
            .collect();
        if let Some(v) = combo(ui, "exp-format", spec.format, &fmts, 64.0) {
            spec.format = v;
        }
        if spec.format == Format::Png {
            let scales = [(1u8, "1x"), (2, "2x"), (3, "3x"), (4, "4x")];
            if let Some(v) = combo(ui, "exp-scale", spec.scale as u8, &scales, 48.0) {
                spec.scale = v as f64;
            }
        }
    });
    let scopes: Vec<(Scope, &str)> = Scope::ALL.iter().map(|s| (*s, s.label())).collect();
    row(ui, "Export", |ui| {
        if let Some(v) = combo(ui, "exp-scope", spec.scope, &scopes, 110.0) {
            spec.scope = v;
        }
    });
    if spec.format != Format::Svg || spec.transparent {
        ui.checkbox(&mut spec.transparent, "Transparent background");
    }
    let what = match (spec.scope, ed.top_selection().len()) {
        (Scope::Selection, 0) => format!("page “{}”", ed.page_name()),
        (Scope::Selection, 1) => ed
            .doc
            .get(ed.top_selection()[0])
            .map_or("selection".into(), |n| format!("“{}”", n.name)),
        (Scope::Selection, n) => format!("{n} layers"),
        (Scope::Frames, _) => "each frame".into(),
        (Scope::Page, _) => format!("page “{}”", ed.page_name()),
        (Scope::AllPages, _) => format!("{} pages", ed.pages.len()),
    };
    if ui
        .add(
            egui::Button::new(format!("Export {what}…"))
                .min_size(egui::vec2(ui.available_width(), 24.0)),
        )
        .clicked()
    {
        ed.requests.push(AppRequest::Export(*spec));
    }
    ui.horizontal(|ui| {
        if ui.small_button("Copy as PNG").clicked() {
            ed.run(crate::editor::shortcuts::Action::CopyAsPng);
        }
        if ui.small_button("Copy as SVG").clicked() {
            ed.run(crate::editor::shortcuts::Action::CopyAsSvg);
        }
    });
}

fn nothing_selected(ui: &mut Ui, ed: &mut Editor) {
    ui.label(title_text("New shapes"));
    let mut sketch = ed.sketch.is_some();
    if ui.checkbox(&mut sketch, "Hand-drawn style").changed() {
        ed.toggle_sketch();
    }
    let routes = [
        (crate::model::Route::Straight, "Straight"),
        (crate::model::Route::Curved, "Curved"),
        (crate::model::Route::Elbow, "Elbow"),
    ];
    row(ui, "Arrows", |ui| {
        if let Some(v) = combo(ui, "new-route", ed.arrow_route, &routes, 96.0) {
            ed.arrow_route = v;
        }
    });
    row(ui, "Sticky", |ui| {
        if let Some(c) = swatches(ui, ed.sticky_color) {
            ed.sticky_color = c;
        }
    });
    separator(ui);
    ui.label(title_text("Pencil"));
    row(ui, "Width", |ui| {
        ui.add(egui::Slider::new(&mut ed.ink_width, 0.5..=24.0))
    });
    row(ui, "Smoothing", |ui| {
        ui.add(egui::Slider::new(&mut ed.ink_smoothing, 0.0..=1.0))
    });
    row(ui, "Color", |ui| {
        let mut c = c32(ed.ink_color);
        if ui.color_edit_button_srgba(&mut c).changed() {
            ed.ink_color = from32(c);
        }
    });
    separator(ui);
    ui.label(title_text("Page"));
    let mut name = ed.page_name().to_string();
    if ui.text_edit_singleline(&mut name).changed() {
        let i = ed.active_page;
        ed.rename_page(i, &name);
    }
    ui.label(
        RichText::new(format!("{} layers", ed.doc.nodes.len()))
            .size(theme::SECONDARY)
            .color(theme::pal(ui).secondary),
    );
}

fn header(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    if ids.len() > 1 {
        ui.label(title_text(format!("{} layers", ids.len())));
        ui.horizontal_wrapped(|ui| {
            for op in [
                BoolOp::Union,
                BoolOp::Subtract,
                BoolOp::Intersect,
                BoolOp::Exclude,
            ] {
                if ui.small_button(op.label()).clicked() {
                    ed.boolean(op);
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.small_button("Group").clicked() {
                ed.group_selection();
            }
            if ui.small_button("Frame").clicked() {
                ed.run(crate::editor::shortcuts::Action::WrapInFrame);
            }
            if ui.small_button("Component").clicked() {
                ed.create_component();
            }
        });
        return;
    }
    let kind = if first.component {
        "Component"
    } else if ed.is_instance_root(first.id) {
        "Instance"
    } else {
        first.kind.type_name()
    };
    ui.label(title_text(format!("{kind} · {}", first.name)));
    ui.horizontal_wrapped(|ui| {
        if first.component {
            if ui.small_button("Create instance").clicked() {
                ed.instance_of_selection();
            }
        } else if ed.is_instance_root(first.id) {
            if ui.small_button("Go to main").clicked() {
                ed.go_to_main();
            }
            if ui.small_button("Detach").clicked() {
                ed.detach_instance();
            }
        } else if (first.is_frame() || first.is_group())
            && ui.small_button("Create component").clicked()
        {
            ed.create_component();
        }
        if first.instance.is_some()
            && first
                .instance
                .as_ref()
                .is_some_and(|l| !l.overrides.is_empty())
            && ui.small_button("Reset overrides").clicked()
        {
            ed.reset_overrides();
        }
        if first.is_container()
            && !first.children.is_empty()
            && ui.small_button("Ungroup").clicked()
        {
            ed.ungroup_selection();
        }
        if !first.is_container()
            && !matches!(first.kind, NodeKind::Path { .. })
            && ui.small_button("Flatten").clicked()
        {
            ed.flatten_selection();
        }
        if matches!(first.kind, NodeKind::Path { .. }) && ui.small_button("Edit points").clicked() {
            ed.start_path_edit(first.id);
        }
    });
    if let Some(l) = &first.instance
        && !l.overrides.is_empty()
    {
        ui.label(
            RichText::new(format!("Overrides: {}", l.overrides.join(", ")))
                .size(theme::SECONDARY)
                .color(theme::pal(ui).secondary),
        );
    }
}

fn align_row(ui: &mut Ui, ed: &mut Editor, _ids: &[NodeId]) {
    use theme::Icon as I;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        for (icon, axis, to, tip) in [
            (I::AlignLeft, 0, 0.0, "Align left (⌥A)"),
            (I::AlignHCenter, 0, 0.5, "Align horizontal centers (⌥H)"),
            (I::AlignRight, 0, 1.0, "Align right (⌥D)"),
            (I::AlignTop, 1, 0.0, "Align top (⌥W)"),
            (I::AlignVCenter, 1, 0.5, "Align vertical centers (⌥V)"),
            (I::AlignBottom, 1, 1.0, "Align bottom (⌥S)"),
        ] {
            if icon_btn(ui, icon, false, tip) {
                ed.align(axis, to);
            }
        }
        ui.add_space(8.0);
        if icon_btn(
            ui,
            I::DistributeH,
            false,
            "Distribute horizontal spacing (⌥⇧H)",
        ) {
            ed.distribute(0);
        }
        if icon_btn(
            ui,
            I::DistributeV,
            false,
            "Distribute vertical spacing (⌥⇧V)",
        ) {
            ed.distribute(1);
        }
    });
}

fn geometry(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    if ids.len() == 1 {
        let r = first.rect;
        ui.horizontal(|ui| {
            if let Some(v) = num_field(ui, ed, "X  ", r.x, 1.0, f64::MIN..=f64::MAX, 92.0) {
                apply(ed, "x", ids, |n| n.rect.x = v);
            }
            if let Some(v) = num_field(ui, ed, "Y  ", r.y, 1.0, f64::MIN..=f64::MAX, 92.0) {
                apply(ed, "y", ids, |n| n.rect.y = v);
            }
        });
        ui.horizontal(|ui| {
            if let Some(v) = num_field(ui, ed, "W  ", r.w, 1.0, 0.0..=1e6, 92.0) {
                resize(ed, ids[0], v2(v, r.h));
            }
            let text = matches!(first.kind, NodeKind::Text { .. });
            ui.add_enabled_ui(!text, |ui| {
                if let Some(v) = num_field(ui, ed, "H  ", r.h, 1.0, 0.0..=1e6, 92.0) {
                    resize(ed, ids[0], v2(r.w, v));
                }
            });
        });
    }
    // Shape parameters.
    match first.kind {
        NodeKind::Polygon { sides } => {
            row(ui, "Sides", |ui| {
                if let Some(v) = num_field(ui, ed, "", sides as f64, 0.1, 3.0..=60.0, 60.0) {
                    apply(ed, "sides", ids, |n| {
                        if let NodeKind::Polygon { sides } = &mut n.kind {
                            *sides = v.round() as u32;
                        }
                    });
                }
            });
        }
        NodeKind::Star { points, inner } => {
            ui.horizontal(|ui| {
                if let Some(v) = num_field(ui, ed, "Points ", points as f64, 0.1, 3.0..=60.0, 92.0)
                {
                    apply(ed, "points", ids, |n| {
                        if let NodeKind::Star { points, .. } = &mut n.kind {
                            *points = v.round() as u32;
                        }
                    });
                }
                if let Some(v) = num_field(ui, ed, "Ratio ", inner * 100.0, 0.5, 1.0..=100.0, 92.0)
                {
                    apply(ed, "ratio", ids, |n| {
                        if let NodeKind::Star { inner, .. } = &mut n.kind {
                            *inner = v / 100.0;
                        }
                    });
                }
            });
        }
        _ => {}
    }
    if matches!(
        first.kind,
        NodeKind::Rect | NodeKind::Frame { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. }
    ) {
        let per_corner = first.style.radii.is_some();
        ui.horizontal(|ui| {
            if !per_corner {
                if let Some(v) = num_field(
                    ui,
                    ed,
                    "Radius  ",
                    first.style.corner_radius,
                    0.5,
                    0.0..=1e4,
                    92.0,
                ) {
                    apply(ed, "radius", ids, |n| n.style.corner_radius = v);
                }
            } else {
                let rr = first.style.corner_radii();
                for (i, p) in ["TL", "TR", "BR", "BL"].iter().enumerate() {
                    if let Some(v) =
                        num_field(ui, ed, &format!("{p} "), rr[i], 0.5, 0.0..=1e4, 46.0)
                    {
                        apply(ed, "radius", ids, |n| {
                            let mut r = n.style.corner_radii();
                            r[i] = v;
                            n.style.radii = Some(r);
                        });
                    }
                }
            }
            if matches!(first.kind, NodeKind::Rect | NodeKind::Frame { .. })
                && icon_btn(ui, theme::Icon::Corners, per_corner, "Independent corners")
            {
                apply(ed, "radius", ids, |n| {
                    n.style.radii = if n.style.radii.is_some() {
                        None
                    } else {
                        Some([n.style.corner_radius; 4])
                    }
                });
            }
        });
    }
    if let NodeKind::Frame { clip } = first.kind {
        let mut c = clip;
        if ui.checkbox(&mut c, "Clip content").changed() {
            apply(ed, "clip", ids, |n| n.kind = NodeKind::Frame { clip: c });
        }
    }
}

/// Resize through the layout engine so constraints and auto layout follow.
fn resize(ed: &mut Editor, id: NodeId, size: crate::geom::V2) {
    let key = format!("size:{id:?}");
    ed.begin_coalescing("Resize", Some(&key));
    {
        let mut tx = ed.tx();
        let tx = &mut tx;
        let old = tx.doc().get(id).unwrap().rect;
        let new = crate::geom::Rect::new(old.x, old.y, size.x.max(0.0), size.y.max(0.0));
        let n = tx.node_mut(id).unwrap();
        n.set_rect_scaling(new);
        if (new.w - old.w).abs() > 1e-9 {
            n.layout_child.sizing[0] = Sizing::Fixed;
        }
        if (new.h - old.h).abs() > 1e-9 {
            n.layout_child.sizing[1] = Sizing::Fixed;
        }
        crate::layout::resize_children(tx, id, old, new);
    }
    ed.mark_overrides(&[id], "size");
    ed.fit_text(id);
    ed.commit();
}

fn child_layout(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    let parent = first.parent.and_then(|p| ed.doc.get(p)).cloned();
    let in_auto = parent.as_ref().is_some_and(|p| p.layout.is_some());
    let sizing_opts_for = |axis: usize| -> Vec<(Sizing, &'static str)> {
        let mut v = vec![(Sizing::Fixed, "Fixed")];
        if first.layout.is_some() || matches!(first.kind, NodeKind::Text { .. }) {
            v.push((Sizing::Hug, "Hug"));
        }
        if in_auto && !first.layout_child.absolute {
            v.push((Sizing::Fill, "Fill"));
        }
        let _ = axis;
        v
    };
    let show_sizing = first.layout.is_some() || (in_auto && !first.layout_child.absolute);
    if show_sizing {
        ui.horizontal(|ui| {
            for (axis, label) in [(0usize, "W"), (1, "H")] {
                ui.label(RichText::new(label).color(theme::pal(ui).secondary));
                let opts = sizing_opts_for(axis);
                if let Some(v) = combo(
                    ui,
                    &format!("sizing{axis}"),
                    first.layout_child.sizing[axis],
                    &opts,
                    66.0,
                ) {
                    apply(ed, "layout", ids, |n| {
                        n.layout_child.sizing[axis] = v;
                        if let NodeKind::Text { auto_width, .. } = &mut n.kind
                            && axis == 0
                        {
                            *auto_width = v == Sizing::Hug;
                        }
                    });
                }
            }
        });
    }
    if in_auto {
        let mut abs = first.layout_child.absolute;
        if ui.checkbox(&mut abs, "Ignore auto layout").changed() {
            apply(ed, "layout", ids, |n| n.layout_child.absolute = abs);
        }
    }
    let constrained =
        parent.as_ref().is_some_and(|p| p.is_frame()) && (!in_auto || first.layout_child.absolute);
    if constrained {
        let opts_h = [
            (Constraint::Min, "Left"),
            (Constraint::Max, "Right"),
            (Constraint::Stretch, "Left & right"),
            (Constraint::Center, "Center"),
            (Constraint::Scale, "Scale"),
        ];
        let opts_v = [
            (Constraint::Min, "Top"),
            (Constraint::Max, "Bottom"),
            (Constraint::Stretch, "Top & bottom"),
            (Constraint::Center, "Center"),
            (Constraint::Scale, "Scale"),
        ];
        row(ui, "Constraints", |ui| {
            if let Some(v) = combo(ui, "con-h", first.layout_child.constraints.h, &opts_h, 74.0) {
                apply(ed, "layout", ids, |n| n.layout_child.constraints.h = v);
            }
            if let Some(v) = combo(ui, "con-v", first.layout_child.constraints.v, &opts_v, 74.0) {
                apply(ed, "layout", ids, |n| n.layout_child.constraints.v = v);
            }
        });
    }
}

fn auto_layout(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    separator(ui);
    ui.horizontal(|ui| {
        ui.label(title_text("Auto layout"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if first.layout.is_some() {
                if ui
                    .small_button("−")
                    .on_hover_text("Remove auto layout (⌥⇧A)")
                    .clicked()
                {
                    ed.remove_auto_layout();
                }
            } else if ui
                .small_button("+")
                .on_hover_text("Add auto layout (⇧A)")
                .clicked()
            {
                ed.add_auto_layout();
            }
        });
    });
    let Some(l) = first.layout else { return };
    let set = |ed: &mut Editor, f: &dyn Fn(&mut AutoLayout)| {
        apply(ed, "layout", ids, |n| {
            if let Some(l) = &mut n.layout {
                f(l);
            }
        });
    };
    ui.horizontal(|ui| {
        if icon_btn(
            ui,
            theme::Icon::ArrowRight,
            l.direction == Direction::Horizontal,
            "Horizontal",
        ) {
            set(ed, &|l| l.direction = Direction::Horizontal);
        }
        if icon_btn(
            ui,
            theme::Icon::ArrowDown,
            l.direction == Direction::Vertical,
            "Vertical",
        ) {
            set(ed, &|l| l.direction = Direction::Vertical);
        }
        if icon_btn(ui, theme::Icon::Wrap, l.wrap, "Wrap") {
            set(ed, &|l| l.wrap = !l.wrap);
        }
        ui.add_space(6.0);
        ui.add_enabled_ui(!l.gap_auto, |ui| {
            if let Some(v) = num_field(ui, ed, "Gap ", l.gap, 0.5, -1e4..=1e4, 70.0) {
                set(ed, &|l| l.gap = v);
            }
        });
        if icon_btn(ui, theme::Icon::SpaceBetween, l.gap_auto, "Space between") {
            set(ed, &|l| l.gap_auto = !l.gap_auto);
        }
    });
    ui.horizontal(|ui| {
        for (i, p) in ["T ", "R ", "B ", "L "].iter().enumerate() {
            if let Some(v) = num_field(ui, ed, p, l.padding[i], 0.5, 0.0..=1e4, 44.0) {
                set(ed, &|l| l.padding[i] = v);
            }
        }
    });
    let opts = [
        (Align::Start, "Start"),
        (Align::Center, "Center"),
        (Align::End, "End"),
    ];
    row(ui, "Align", |ui| {
        if let Some(v) = combo(ui, "al-main", l.align_main, &opts, 64.0) {
            set(ed, &|l| l.align_main = v);
        }
        if let Some(v) = combo(ui, "al-cross", l.align_cross, &opts, 64.0) {
            set(ed, &|l| l.align_cross = v);
        }
    });
}

fn text_section(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    let NodeKind::Text {
        font_size, align, ..
    } = &first.kind
    else {
        return;
    };
    let editing_range = ed.text_selection().is_some();
    ui.horizontal(|ui| {
        ui.label(title_text("Text"));
        if editing_range {
            ui.label(
                RichText::new("selection")
                    .size(theme::SECONDARY)
                    .color(theme::pal(ui).secondary),
            );
        }
    });
    let cs = ed.current_char_style();
    ui.horizontal(|ui| {
        let size = cs.map_or(*font_size, |c| c.size);
        if let Some(v) = num_field(ui, ed, "Size ", size, 0.5, 1.0..=1000.0, 80.0) {
            ed.style_text("font size", |s| s.size = Some(v));
        }
        let c = cs.map(|c| c.color).unwrap_or(Color::INK);
        if let Some(nc) = color_button(ui, ed, c) {
            ed.style_text("color", |s| s.color = Some(nc));
        }
        ui.add_space(4.0);
        if icon_toggle(ui, "B", cs.is_some_and(|c| c.bold), "Bold (⌘B)") {
            ed.toggle_bold();
        }
        if icon_toggle(ui, "I", cs.is_some_and(|c| c.italic), "Italic (⌘I)") {
            ed.toggle_italic();
        }
        if icon_toggle(ui, "U", cs.is_some_and(|c| c.underline), "Underline (⌘U)") {
            ed.toggle_underline();
        }
        if icon_toggle(ui, "S", cs.is_some_and(|c| c.strike), "Strikethrough (⌘⇧X)") {
            ed.toggle_strike();
        }
    });
    ui.horizontal(|ui| {
        for (a, ic) in [
            (TextAlign::Left, theme::Icon::AlignLeft),
            (TextAlign::Center, theme::Icon::AlignHCenter),
            (TextAlign::Right, theme::Icon::AlignRight),
        ] {
            if icon_btn(ui, ic, *align == a, "Text alignment") {
                apply(ed, "align", ids, |n| {
                    if let NodeKind::Text { align, .. } = &mut n.kind {
                        *align = a;
                    }
                });
            }
        }
        ui.add_space(6.0);
        let lh = first.text_style.line_height.unwrap_or(0.0) * 100.0;
        if let Some(v) = num_field(ui, ed, "LH ", lh, 1.0, 0.0..=1000.0, 64.0) {
            apply(ed, "line height", ids, |n| {
                n.text_style.line_height = (v > 0.0).then_some(v / 100.0)
            });
        }
        if let Some(v) = num_field(
            ui,
            ed,
            "LS ",
            first.text_style.letter_spacing,
            0.1,
            -50.0..=200.0,
            60.0,
        ) {
            apply(ed, "letter spacing", ids, |n| {
                n.text_style.letter_spacing = v
            });
        }
    });
    if ids.len() == 1 && ed.text_edit.is_none() {
        let NodeKind::Text { text, .. } = &first.kind else {
            return;
        };
        let mut t = text.clone();
        let r = ui.add(
            egui::TextEdit::multiline(&mut t)
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        if r.changed() {
            apply(ed, "text", ids, |n| {
                if let NodeKind::Text { text, .. } = &mut n.kind {
                    *text = t.clone();
                }
                n.text_style.spans.retain(|s| s.end <= t.chars().count());
            });
        }
        seal_on_release(ed, &r);
    }
}

fn fills(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    let fills = first.style.fills.clone();
    ui.horizontal(|ui| {
        ui.label(title_text("Fill"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("+").on_hover_text("Add fill").clicked() {
                let c = fills
                    .last()
                    .map_or(Color::rgb(0xd9, 0xd9, 0xd9), |p| p.preview_color());
                apply(ed, "fills", ids, |n| n.style.fills.push(Paint::solid(c)));
            }
        });
    });
    // Top fill first, as in Figma.
    for i in (0..fills.len()).rev() {
        let p = fills[i].clone();
        ui.push_id(i, |ui| fill_row(ui, ed, ids, i, &p));
    }
}

fn fill_row(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], i: usize, p: &Paint) {
    let set = |ed: &mut Editor, np: Paint| {
        apply(ed, "fills", ids, |n| {
            if let Some(slot) = n.style.fills.get_mut(i) {
                *slot = np.clone();
            }
        });
    };
    ui.horizontal(|ui| {
        let kinds = [("Solid", 0u8), ("Linear", 1), ("Radial", 2), ("Image", 3)];
        let cur = match p {
            Paint::Solid { .. } => 0,
            Paint::Linear { .. } => 1,
            Paint::Radial { .. } => 2,
            Paint::Image { .. } => 3,
        };
        let opts: Vec<(u8, &str)> = kinds.iter().map(|(l, k)| (*k, *l)).collect();
        if let Some(k) = combo(ui, "kind", cur, &opts, 62.0) {
            let c = p.preview_color();
            let np = match k {
                0 => Paint::solid(c),
                1 => Paint::linear(c, Color::rgba(c.r, c.g, c.b, 0)),
                2 => Paint::radial(c, Color::rgba(c.r, c.g, c.b, 0)),
                _ => {
                    ed.requests.push(AppRequest::ImageFill(ids.to_vec(), i));
                    return;
                }
            };
            set(ed, np);
            return;
        }
        match p {
            Paint::Solid { color } => {
                if let Some(c) = color_button(ui, ed, *color) {
                    set(ed, Paint::solid(c));
                }
                ui.label(RichText::new(color.to_hex()).monospace().weak());
            }
            Paint::Image {
                mode,
                opacity,
                asset,
            } => {
                let modes = [
                    (ImageMode::Fill, "Fill"),
                    (ImageMode::Fit, "Fit"),
                    (ImageMode::Stretch, "Stretch"),
                    (ImageMode::Tile, "Tile"),
                ];
                if let Some(m) = combo(ui, "mode", *mode, &modes, 62.0) {
                    set(
                        ed,
                        Paint::Image {
                            asset: asset.clone(),
                            mode: m,
                            opacity: *opacity,
                        },
                    );
                }
                if ui.small_button("Replace…").clicked() {
                    ed.requests.push(AppRequest::ImageFill(ids.to_vec(), i));
                }
            }
            _ => {}
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("−").on_hover_text("Remove fill").clicked() {
                apply(ed, "fills", ids, |n| {
                    if i < n.style.fills.len() {
                        n.style.fills.remove(i);
                    }
                });
            }
        });
    });
    // Gradient stops and geometry.
    if let Paint::Linear { stops, from, to }
    | Paint::Radial {
        stops,
        center: from,
        radius: to,
    } = p
    {
        let linear = matches!(p, Paint::Linear { .. });
        let rebuild = |stops: Vec<Stop>, a: crate::geom::V2, b: crate::geom::V2| {
            if linear {
                Paint::Linear {
                    stops,
                    from: a,
                    to: b,
                }
            } else {
                Paint::Radial {
                    stops,
                    center: a,
                    radius: b,
                }
            }
        };
        for (si, s) in stops.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(LABEL_W * 0.5);
                if let Some(c) = color_button(ui, ed, s.color) {
                    let mut ns = stops.clone();
                    ns[si].color = c;
                    set(ed, rebuild(ns, *from, *to));
                }
                if let Some(v) = num_field(ui, ed, "", s.pos * 100.0, 0.5, 0.0..=100.0, 56.0) {
                    let mut ns = stops.clone();
                    ns[si].pos = v / 100.0;
                    set(ed, rebuild(ns, *from, *to));
                }
                ui.label(RichText::new("%").weak());
                if stops.len() > 2 && ui.small_button("−").clicked() {
                    let mut ns = stops.clone();
                    ns.remove(si);
                    set(ed, rebuild(ns, *from, *to));
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W * 0.5);
            if ui.small_button("+ stop").clicked() {
                let mut ns = stops.clone();
                let c = ns.last().map_or(Color::BLACK, |s| s.color);
                ns.push(Stop { pos: 0.5, color: c });
                ns.sort_by(|a, b| a.pos.total_cmp(&b.pos));
                set(ed, rebuild(ns, *from, *to));
            }
            if linear {
                let d = *to - *from;
                let ang = d.y.atan2(d.x).to_degrees();
                if let Some(v) = num_field(ui, ed, "Angle ", ang, 1.0, -360.0..=360.0, 70.0) {
                    let a = v.to_radians();
                    let half = v2(a.cos(), a.sin()) * 0.5;
                    let c = v2(0.5, 0.5);
                    set(ed, rebuild(stops.clone(), c - half, c + half));
                }
            } else if let Some(v) = num_field(ui, ed, "R ", to.x * 100.0, 0.5, 1.0..=400.0, 70.0) {
                set(ed, rebuild(stops.clone(), *from, v2(v / 100.0, v / 100.0)));
            }
        });
    }
}

fn stroke(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    let s = &first.style;
    ui.horizontal(|ui| {
        ui.label(title_text("Stroke"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if s.stroke.is_some() {
                if ui.small_button("−").clicked() {
                    apply(ed, "stroke", ids, |n| n.style.stroke = None);
                }
            } else if ui.small_button("+").clicked() {
                apply(ed, "stroke", ids, |n| n.style.stroke = Some(Color::INK));
            }
        });
    });
    let Some(c) = s.stroke else { return };
    ui.horizontal(|ui| {
        if let Some(nc) = color_button(ui, ed, c) {
            apply(ed, "stroke", ids, |n| n.style.stroke = Some(nc));
        }
        if let Some(v) = num_field(ui, ed, "W ", s.stroke_width, 0.1, 0.0..=500.0, 64.0) {
            apply(ed, "stroke width", ids, |n| n.style.stroke_width = v);
        }
        let al = [
            (StrokeAlign::Inside, "Inside"),
            (StrokeAlign::Center, "Center"),
            (StrokeAlign::Outside, "Outside"),
        ];
        if let Some(v) = combo(ui, "salign", s.stroke_align, &al, 64.0) {
            apply(ed, "stroke align", ids, |n| n.style.stroke_align = v);
        }
    });
    ui.horizontal(|ui| {
        let dash_kind = match s.dash.as_slice() {
            [] => 0u8,
            [a, _] if *a <= 2.0 => 2,
            _ => 1,
        };
        let dashes = [(0u8, "Solid"), (1, "Dashed"), (2, "Dotted")];
        if let Some(k) = combo(ui, "dash", dash_kind, &dashes, 64.0) {
            let w = s.stroke_width.max(1.0);
            apply(ed, "dash", ids, |n| {
                n.style.dash = match k {
                    0 => vec![],
                    1 => vec![w * 4.0, w * 3.0],
                    _ => vec![w.min(2.0), w * 2.5],
                };
                if k == 2 {
                    n.style.cap = Cap::Round;
                }
            });
        }
        let caps = [
            (Cap::Butt, "Butt"),
            (Cap::Round, "Round"),
            (Cap::Square, "Square"),
        ];
        if let Some(v) = combo(ui, "cap", s.cap, &caps, 60.0) {
            apply(ed, "cap", ids, |n| n.style.cap = v);
        }
        let joins = [
            (Join::Miter, "Miter"),
            (Join::Round, "Round"),
            (Join::Bevel, "Bevel"),
        ];
        if let Some(v) = combo(ui, "join", s.join, &joins, 60.0) {
            apply(ed, "join", ids, |n| n.style.join = v);
        }
    });
}

fn layer(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    ui.label(title_text("Layer"));
    ui.horizontal(|ui| {
        let opts: Vec<(BlendMode, &str)> = BlendMode::ALL.iter().map(|b| (*b, b.label())).collect();
        if let Some(v) = combo(ui, "blend", first.style.blend, &opts, 96.0) {
            apply(ed, "blend", ids, |n| n.style.blend = v);
        }
        let mut op = first.style.opacity * 100.0;
        let r = ui.add_sized(
            egui::vec2(70.0, 20.0),
            egui::DragValue::new(&mut op)
                .range(0.0..=100.0)
                .suffix("%")
                .speed(0.5),
        );
        if r.changed() {
            apply(ed, "opacity", ids, |n| n.style.opacity = op / 100.0);
        }
        seal_on_release(ed, &r);
    });
}
