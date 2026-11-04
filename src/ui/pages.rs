//! The pages list at the top of the left sidebar (as in Figma): switch, add, rename,
//! duplicate, delete and drag to reorder.

use super::{UiState, section};
use crate::editor::Editor;
use crate::theme::{self, Icon};
use egui::{Color32, RichText, Ui};

#[derive(Default)]
pub struct PagesUi {
    pub collapsed: bool,
    pub renaming: Option<(usize, String)>,
    /// Row being dragged, and the index it would drop at.
    drag: Option<(usize, usize)>,
}

const ROW_H: f32 = 24.0;

pub fn panel(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    let p = theme::pal(ui);
    ui.horizontal(|ui| {
        let chevron = if st.pages.collapsed {
            Icon::ChevronRight
        } else {
            Icon::ChevronDown
        };
        if theme::icon_button(ui, chevron, 16.0, false).clicked() {
            st.pages.collapsed = !st.pages.collapsed;
        }
        section(ui, "Pages");
        if st.pages.collapsed {
            ui.label(
                RichText::new(ed.page_name())
                    .size(theme::SECONDARY)
                    .color(p.secondary),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::icon_button(ui, Icon::Plus, 20.0, false)
                .on_hover_text("Add page")
                .clicked()
            {
                ed.add_page();
            }
        });
    });
    if st.pages.collapsed {
        return;
    }
    let n = ed.pages.len();
    let max_h = (n as f32 * (ROW_H + 1.0)).min(6.5 * (ROW_H + 1.0));
    egui::ScrollArea::vertical()
        .id_salt("pages")
        .max_height(max_h)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let top = ui.cursor().top();
            for i in 0..n {
                page_row(ui, ed, st, i, top);
            }
        });
    // Drop: reorder.
    if let Some((from, to)) = st.pages.drag
        && ui.input(|i| i.pointer.any_released())
    {
        st.pages.drag = None;
        ed.move_page(from, to.min(n - 1));
    }
}

fn page_row(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, i: usize, top: f32) {
    let p = theme::pal(ui);
    let active = i == ed.active_page;
    let (row, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_H),
        egui::Sense::click_and_drag(),
    );
    if active {
        ui.painter().rect_filled(row, 6.0, p.pressed);
    } else if resp.hovered() {
        ui.painter().rect_filled(row, 6.0, p.hover);
    }
    // Drop indicator while dragging.
    if let Some((from, to)) = st.pages.drag
        && from != to
        && to == i
    {
        let y = if to > from { row.bottom() } else { row.top() };
        ui.painter()
            .hline(row.x_range(), y, egui::Stroke::new(2.0, theme::accent()));
    }
    theme::paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(row.left() + 14.0, row.center().y),
            egui::vec2(13.0, 13.0),
        ),
        Icon::Page,
        if active { theme::accent() } else { p.secondary },
    );
    let name_rect = egui::Rect::from_min_max(
        egui::pos2(row.left() + 28.0, row.top()),
        egui::pos2(row.right() - 4.0, row.bottom()),
    );
    let renaming = st.pages.renaming.as_ref().is_some_and(|(r, _)| *r == i);
    if renaming {
        let (_, name) = st.pages.renaming.as_mut().unwrap();
        let r = ui.put(
            name_rect.shrink2(egui::vec2(0.0, 2.0)),
            egui::TextEdit::singleline(name),
        );
        r.request_focus();
        if r.lost_focus() {
            let (idx, name) = st.pages.renaming.take().unwrap();
            if !ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                ed.rename_page(idx, &name);
            }
        }
    } else {
        let font = if active {
            theme::medium(theme::BODY)
        } else {
            egui::FontId::proportional(theme::BODY)
        };
        let g = ui
            .painter()
            .layout(ed.pages[i].name.clone(), font, p.text, f32::INFINITY);
        ui.painter()
            .with_clip_rect(name_rect.intersect(ui.clip_rect()))
            .galley(
                egui::pos2(name_rect.left(), row.center().y - g.size().y / 2.0),
                g,
                Color32::PLACEHOLDER,
            );
    }
    if resp.double_clicked() {
        st.pages.renaming = Some((i, ed.pages[i].name.clone()));
    } else if resp.clicked() {
        ed.switch_page(i);
    }
    if resp.drag_started() {
        st.pages.drag = Some((i, i));
    }
    if resp.dragged()
        && let Some(pos) = ui.input(|inp| inp.pointer.interact_pos())
        && let Some((from, _)) = st.pages.drag
    {
        let to = ((pos.y - top) / (ROW_H + 1.0)).floor().max(0.0) as usize;
        st.pages.drag = Some((from, to.min(ed.pages.len() - 1)));
    }
    resp.context_menu(|ui| {
        if ui.button("Rename").clicked() {
            st.pages.renaming = Some((i, ed.pages[i].name.clone()));
            ui.close();
        }
        if ui.button("Duplicate").clicked() {
            ed.duplicate_page(i);
            ui.close();
        }
        if ui
            .add_enabled(ed.pages.len() > 1, egui::Button::new("Delete"))
            .clicked()
        {
            ed.delete_page(i);
            ui.close();
        }
    });
}
