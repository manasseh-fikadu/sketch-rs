//! The Library tab: reusable shapes with vector previews; click to insert, add the selection,
//! rename or delete items.

use super::UiState;
use crate::editor::Editor;
use crate::library::{LibItem, Library};
use crate::render::draw;
use crate::render::tess::Tess;
use crate::theme::{self, Icon};
use egui::{Color32, RichText, Ui};
use std::collections::HashMap;

#[derive(Default)]
pub struct LibraryUi {
    pub library: Library,
    /// The library changed and should be saved.
    pub dirty: bool,
    pub renaming: Option<(String, String)>,
    previews: HashMap<String, (usize, egui::Mesh)>,
}

const TILE: f32 = 92.0;

/// Tessellate an item into an egui mesh fitted in a `size` box at the origin.
fn preview_mesh(item: &LibItem, size: f32) -> egui::Mesh {
    let doc = item.doc();
    let mut mesh = egui::Mesh::default();
    let Some(b) = item.bounds() else { return mesh };
    let s = ((size as f64 - 12.0) / b.w.max(b.h).max(1.0)).min(1.0);
    let mut tess = Tess::new(s.max(0.05));
    for id in doc.paint_order() {
        let (Some(n), Some(wr)) = (doc.get(id), doc.world_rect(id)) else {
            continue;
        };
        if !n.visible {
            continue;
        }
        let d = draw::node_draw(n, wr, draw::inherited_opacity(&doc, id) * n.style.opacity);
        for piece in tess.ops(&d.ops) {
            let base = mesh.vertices.len() as u32;
            for v in &piece.mesh.vertices {
                let x = ((v.pos[0] as f64 - b.x) * s) as f32 + (size - (b.w * s) as f32) * 0.5;
                let y = ((v.pos[1] as f64 - b.y) * s) as f32 + (size - (b.h * s) as f32) * 0.5;
                let c = if v.kind == 0 {
                    v.color
                } else {
                    // Gradients and images preview as a neutral tone.
                    [0.75, 0.75, 0.8, v.color[3]]
                };
                let to8 = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: egui::pos2(x, y),
                    uv: egui::epaint::WHITE_UV,
                    color: Color32::from_rgba_unmultiplied(
                        to8(c[0]),
                        to8(c[1]),
                        to8(c[2]),
                        to8(c[3]),
                    ),
                });
            }
            mesh.indices
                .extend(piece.mesh.indices.iter().map(|i| i + base));
        }
    }
    mesh
}

pub fn panel(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    let p = theme::pal(ui);
    let lib = &mut st.library;
    ui.horizontal(|ui| {
        let can_add = !ed.top_selection().is_empty();
        if ui
            .add_enabled(can_add, egui::Button::new("+ Add selection"))
            .on_hover_text("Save the selected layers as a reusable item")
            .clicked()
        {
            let ids = ed.top_selection();
            let name = ids
                .first()
                .and_then(|id| ed.doc.get(*id))
                .map_or("Item".to_string(), |n| n.name.clone());
            if let Some(item) = LibItem::from_selection(&ed.doc, &ids, &name) {
                lib.library.items.push(item);
                lib.dirty = true;
            }
        }
    });
    ui.add_space(4.0);
    if lib.library.items.is_empty() {
        ui.label(
            RichText::new("Select layers and choose\n“Add selection” to keep them here.")
                .size(theme::SECONDARY)
                .color(p.secondary),
        );
        return;
    }
    let cols = ((ui.available_width() + 6.0) / (TILE + 6.0))
        .floor()
        .max(1.0) as usize;
    let items = lib.library.items.clone();
    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt("library")
        .auto_shrink(false)
        .show(ui, |ui| {
            for row in items.chunks(cols) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for it in row {
                        ui.vertical(|ui| {
                            ui.set_width(TILE);
                            let (rect, resp) = ui
                                .allocate_exact_size(egui::vec2(TILE, TILE), egui::Sense::click());
                            let bg = if resp.hovered() { p.hover } else { p.control };
                            ui.painter().rect_filled(rect, 8.0, bg);
                            ui.painter().rect_stroke(
                                rect,
                                8.0,
                                egui::Stroke::new(1.0, p.separator),
                                egui::StrokeKind::Inside,
                            );
                            let key = it.id.clone();
                            let fp = it.nodes.len() ^ (it.name.len() << 16);
                            let entry = lib
                                .previews
                                .entry(key)
                                .or_insert_with(|| (usize::MAX, egui::Mesh::default()));
                            if entry.0 != fp {
                                *entry = (fp, preview_mesh(it, TILE));
                            }
                            let mut m = entry.1.clone();
                            m.translate(rect.min.to_vec2());
                            ui.painter().add(egui::Shape::mesh(m));
                            let renaming =
                                lib.renaming.as_ref().is_some_and(|(id, _)| *id == it.id);
                            if renaming {
                                let (_, name) = lib.renaming.as_mut().unwrap();
                                let r =
                                    ui.add(egui::TextEdit::singleline(name).desired_width(TILE));
                                r.request_focus();
                                if r.lost_focus() {
                                    let (id, name) = lib.renaming.take().unwrap();
                                    if let Some(x) =
                                        lib.library.items.iter_mut().find(|x| x.id == id)
                                    {
                                        x.name = name;
                                        lib.dirty = true;
                                    }
                                }
                            } else {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&it.name).size(theme::SECONDARY),
                                    )
                                    .truncate(),
                                );
                            }
                            if resp.clicked() {
                                let at = ed.camera.screen_to_world(ed.viewport * 0.5);
                                ed.insert_library_item(it, at);
                            }
                            resp.on_hover_text("Click to insert").context_menu(|ui| {
                                if ui.button("Insert").clicked() {
                                    let at = ed.camera.screen_to_world(ed.viewport * 0.5);
                                    ed.insert_library_item(it, at);
                                    ui.close();
                                }
                                if ui.button("Rename").clicked() {
                                    lib.renaming = Some((it.id.clone(), it.name.clone()));
                                    ui.close();
                                }
                                if ui.button("Remove from library").clicked() {
                                    remove = Some(it.id.clone());
                                    ui.close();
                                }
                            });
                        });
                    }
                });
                ui.add_space(6.0);
            }
        });
    if let Some(id) = remove {
        lib.library.items.retain(|x| x.id != id);
        lib.dirty = true;
    }
    let _ = Icon::Library;
}
