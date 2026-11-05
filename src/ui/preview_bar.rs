//! The bar above a running preview: leave, back, restart, jump to any screen, and how to show it.

use super::UiState;
use crate::editor::Editor;
use crate::editor::shortcuts::{self, Action};
use crate::model::{Node, NodeId};
use crate::preview::Scale;
use crate::theme::{self, Icon};
use egui::{RichText, Ui};
use std::time::Duration;

/// Lay out the preview's chrome and return the canvas rectangle.
pub fn draw(ui: &mut Ui, ed: &mut Editor, st: &UiState) -> egui::Rect {
    if !st.hide_ui {
        let p = theme::pal(ui);
        egui::Panel::top("preview-bar")
            .exact_size(if st.native_chrome {
                theme::TOOLBAR_H
            } else {
                34.0
            })
            .frame(
                egui::Frame::NONE
                    .fill(p.window)
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(ui, |ui| bar(ui, ed, st));
    }
    let canvas = ui.available_rect_before_wrap();
    if let Some(p) = &ed.preview {
        if p.cursor_hand {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        // Delayed actions need a frame at the right moment even if nothing else happens.
        if let Some(ms) = p.next_wake(ed.now_ms) {
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64(ms / 1000.0));
        }
    }
    canvas
}

fn bar(ui: &mut Ui, ed: &mut Editor, st: &UiState) {
    let Some(p) = &ed.preview else { return };
    let (can_back, has_interactions, mut hotspots, mut scale) = (
        p.can_go_back(),
        p.has_interactions,
        p.show_hotspots,
        p.scale,
    );
    let (cur_page, cur_frame) = (p.current.page.clone(), p.current.frame);
    let pal = theme::pal(ui);
    ui.horizontal_centered(|ui| {
        if st.native_chrome {
            ui.add_space(theme::TRAFFIC_LIGHTS_W);
        }
        ui.spacing_mut().item_spacing.x = 2.0;
        let close = theme::icon_button(ui, Icon::Close, 26.0, false).on_hover_text(format!(
            "Back to the editor (Esc or {})",
            shortcuts::chord_text(Action::TogglePreview)
        ));
        if close.clicked() {
            ed.stop_preview();
            return;
        }
        ui.add_enabled_ui(can_back, |ui| {
            if theme::icon_button(ui, Icon::ChevronLeft, 26.0, false)
                .on_hover_text("Back (←)")
                .clicked()
            {
                ed.with_preview(|p, ed| p.back(ed));
            }
        });
        if theme::icon_button(ui, Icon::Restore, 26.0, false)
            .on_hover_text("Restart (R)")
            .clicked()
        {
            ed.with_preview(|p, ed| p.restart(ed));
        }
        ui.add_space(8.0);
        if let Some((page, node)) = screen_picker(ui, ed, &cur_page, cur_frame) {
            ed.with_preview(|p, ed| {
                let target = crate::model::Target {
                    page: Some(page.clone()),
                    node,
                };
                p.navigate(ed, &page, &target, Default::default());
            });
        }
        if !has_interactions {
            ui.add_space(10.0);
            ui.label(
                RichText::new("No links on this screen yet. Add them in the Prototype panel.")
                    .size(theme::SECONDARY)
                    .color(pal.secondary),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let options = [(Scale::Fit, "Fit width"), (Scale::Actual, "100%")];
            let label = options
                .iter()
                .find(|(s, _)| *s == scale)
                .map_or("", |(_, l)| *l);
            egui::ComboBox::from_id_salt("preview-scale")
                .selected_text(label)
                .width(84.0)
                .show_ui(ui, |ui| {
                    for (s, l) in options {
                        ui.selectable_value(&mut scale, s, l);
                    }
                });
            ui.add_space(8.0);
            ui.checkbox(&mut hotspots, "Show hotspots");
        });
    });
    // The close button may have ended the preview above.
    if let Some(p) = &mut ed.preview {
        p.show_hotspots = hotspots;
        if p.scale != scale {
            p.set_scale(scale);
        }
    }
    let r = ui.max_rect();
    ui.painter().hline(
        r.x_range(),
        r.bottom() - 0.5,
        egui::Stroke::new(1.0, pal.separator),
    );
}

/// Every screen in the project, by page. Returns the one picked.
fn screen_picker(
    ui: &mut Ui,
    ed: &Editor,
    cur_page: &str,
    cur_frame: NodeId,
) -> Option<(String, NodeId)> {
    let current = ed
        .pages
        .iter()
        .enumerate()
        .find(|(_, s)| s.id == cur_page)
        .and_then(|(i, _)| ed.page_doc(i).get(cur_frame))
        .map_or(String::new(), |n| n.name.clone());
    let secondary = theme::pal(ui).secondary;
    let mut picked = None;
    egui::ComboBox::from_id_salt("preview-screen")
        .selected_text(RichText::new(current).font(theme::medium(theme::BODY)))
        .width(220.0)
        .height(520.0)
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
                    let on = slot.id == cur_page && f.id == cur_frame;
                    if ui.selectable_label(on, &f.name).clicked() && !on {
                        picked = Some((slot.id.clone(), f.id));
                    }
                }
            }
        });
    picked
}
