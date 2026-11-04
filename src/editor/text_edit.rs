//! Text editing: caret and selection, typing with style-span bookkeeping, and per-range rich
//! text styling (bold, italic, underline, strikethrough, size, color).

use super::{Editor, Key, TextEdit};
use crate::model::{Color, NodeId, NodeKind, TextSpan};
use crate::text;

impl Editor {
    fn text_len(&self, id: NodeId) -> usize {
        self.doc
            .get(id)
            .and_then(|n| n.kind.text())
            .map_or(0, |t| t.chars().count())
    }

    /// Selected char range `[a, b)` of the open text edit, if any.
    pub fn text_selection(&self) -> Option<(usize, usize)> {
        let te = self.text_edit.as_ref()?;
        let a = te.anchor?;
        (a != te.caret).then(|| (a.min(te.caret), a.max(te.caret)))
    }

    /// Replace the selection (or insert at the caret), keeping style spans attached.
    pub fn insert_text(&mut self, s: &str) {
        let Some(te) = self.text_edit.clone() else {
            return;
        };
        let sel = self.text_selection();
        let n_new = s.chars().count();
        let mut caret = te.caret;
        if let Some(n) = self.tx().node_mut(te.id)
            && let Some(text) = n.kind.text_mut()
        {
            // Replacement text takes the style of the first replaced character.
            let mut inherit: Option<TextSpan> = None;
            if let Some((a, b)) = sel {
                inherit = n
                    .text_style
                    .spans
                    .iter()
                    .find(|sp| sp.start <= a && sp.end > a)
                    .cloned();
                let (ba, bb) = (byte_at(text, a), byte_at(text, b));
                text.replace_range(ba..bb, "");
                n.text_style.on_delete(a, b);
                caret = a;
            }
            let byte = byte_at(text, caret);
            text.insert_str(byte, s);
            n.text_style.on_insert(caret, n_new);
            if let Some(sp) = inherit {
                n.text_style.apply_range(caret, caret + n_new, |t| {
                    *t = TextSpan {
                        start: t.start,
                        end: t.end,
                        ..sp.clone()
                    }
                });
            }
            caret += n_new;
        }
        self.text_edit = Some(TextEdit {
            id: te.id,
            caret,
            anchor: None,
        });
        self.mark_overrides(&[te.id], "text");
        self.fit_text(te.id);
    }

    fn delete_text(&mut self, a: usize, b: usize) {
        let Some(te) = self.text_edit.clone() else {
            return;
        };
        if a >= b {
            return;
        }
        if let Some(n) = self.tx().node_mut(te.id)
            && let Some(text) = n.kind.text_mut()
        {
            let (ba, bb) = (byte_at(text, a), byte_at(text, b));
            text.replace_range(ba..bb, "");
            n.text_style.on_delete(a, b);
        }
        self.text_edit = Some(TextEdit {
            id: te.id,
            caret: a,
            anchor: None,
        });
        self.mark_overrides(&[te.id], "text");
        self.fit_text(te.id);
    }

    fn move_caret(&mut self, to: usize) {
        let extend = self.mods.shift;
        if let Some(te) = &mut self.text_edit {
            if extend {
                te.anchor.get_or_insert(te.caret);
            } else {
                te.anchor = None;
            }
            te.caret = to;
        }
    }

    /// Caret index one line up/down, keeping the x position.
    fn vertical_caret(&self, down: bool) -> Option<usize> {
        let te = self.text_edit.as_ref()?;
        let l = text::layout_node(self.doc.get(te.id)?)?;
        let li = l.line_of(te.caret);
        let (x, _, _) = l.caret(te.caret);
        let target = if down {
            if li + 1 >= l.lines.len() {
                return Some(l.lines.last()?.end);
            }
            li + 1
        } else {
            if li == 0 {
                return Some(0);
            }
            li - 1
        };
        let line = &l.lines[target];
        Some(l.hit(crate::geom::v2(x, line.top + line.height * 0.5)))
    }

    /// Handle a key while editing text. Returns true when consumed.
    pub(super) fn text_key(&mut self, key: &Key) -> bool {
        let Some(te) = self.text_edit.clone() else {
            return false;
        };
        let len = self.text_len(te.id);
        let cmd = self.mods.command();
        match key {
            Key::Escape => self.end_text_edit(),
            Key::Enter if cmd => self.end_text_edit(),
            Key::Enter => self.insert_text("\n"),
            Key::Backspace => match self.text_selection() {
                Some((a, b)) => self.delete_text(a, b),
                None if te.caret > 0 => {
                    let a = if cmd { 0 } else { te.caret - 1 };
                    self.delete_text(a, te.caret)
                }
                None => {}
            },
            Key::Delete => match self.text_selection() {
                Some((a, b)) => self.delete_text(a, b),
                None if te.caret < len => self.delete_text(te.caret, te.caret + 1),
                None => {}
            },
            Key::Left | Key::Right => {
                let to = match (self.text_selection(), self.mods.shift) {
                    (Some((a, _)), false) if *key == Key::Left => a,
                    (Some((_, b)), false) => b,
                    _ if cmd && *key == Key::Left => 0,
                    _ if cmd => len,
                    _ if *key == Key::Left => te.caret.saturating_sub(1),
                    _ => (te.caret + 1).min(len),
                };
                self.move_caret(to);
            }
            Key::Home | Key::PageUp => self.move_caret(0),
            Key::End | Key::PageDown => self.move_caret(len),
            Key::Up | Key::Down => {
                if let Some(to) = self.vertical_caret(*key == Key::Down) {
                    self.move_caret(to);
                }
            }
            Key::Char('a') | Key::Char('A') if cmd => {
                if let Some(te) = &mut self.text_edit {
                    te.anchor = Some(0);
                    te.caret = len;
                }
            }
            Key::Space | Key::Char(_) | Key::Tab => {}
        }
        true
    }

    /// Apply a style change to the selected text range, or to whole text layers when not
    /// editing (or editing without a selection).
    pub fn style_text(&mut self, field: &str, f: impl Fn(&mut TextSpan)) {
        if let (Some(te), Some((a, b))) = (self.text_edit.clone(), self.text_selection()) {
            {
                let mut tx = self.tx();
                if let Some(n) = tx.node_mut(te.id) {
                    n.text_style.apply_range(a, b, &f);
                }
            }
            self.mark_overrides(&[te.id], field);
            self.fit_text(te.id);
            return;
        }
        let ids: Vec<NodeId> = match &self.text_edit {
            Some(te) => vec![te.id],
            None => self
                .selection
                .iter()
                .copied()
                .filter(|id| {
                    matches!(
                        self.doc.get(*id).map(|n| &n.kind),
                        Some(NodeKind::Text { .. })
                    )
                })
                .collect(),
        };
        if ids.is_empty() {
            return;
        }
        let editing = self.text_edit.is_some();
        if !editing {
            self.begin(&format!("Change {field}"));
        }
        for id in &ids {
            let len = self.text_len(*id);
            let mut tx = self.tx();
            let n = tx.node_mut(*id).unwrap();
            // Whole-layer changes go to the base style where possible.
            let mut probe = TextSpan::default();
            f(&mut probe);
            if let Some(b) = probe.bold {
                n.text_style.bold = b;
            }
            if let Some(i) = probe.italic {
                n.text_style.italic = i;
            }
            if let Some(c) = probe.color {
                n.style.set_fill(Some(c));
            }
            if let Some(z) = probe.size
                && let NodeKind::Text { font_size, .. } | NodeKind::Sticky { font_size, .. } =
                    &mut n.kind
            {
                *font_size = z;
            }
            // Drop span values that the whole-layer change supersedes.
            for sp in &mut n.text_style.spans {
                if probe.bold.is_some() {
                    sp.bold = None;
                }
                if probe.italic.is_some() {
                    sp.italic = None;
                }
                if probe.color.is_some() {
                    sp.color = None;
                }
                if probe.size.is_some() {
                    sp.size = None;
                }
            }
            if probe.underline.is_some() || probe.strike.is_some() {
                n.text_style.apply_range(0, len, &f);
            }
            n.text_style.normalize();
            self.fit_text(*id);
        }
        self.mark_overrides(&ids, field);
        if !editing {
            self.commit();
        }
    }

    /// The style at the caret / selection start (for toggles and the inspector).
    pub fn current_char_style(&self) -> Option<crate::model::CharStyle> {
        let id = self
            .text_edit
            .as_ref()
            .map(|t| t.id)
            .or_else(|| self.selection.first().copied())?;
        let n = self.doc.get(id)?;
        let font_size = &n.kind.font_size()?;
        let at = match (self.text_selection(), &self.text_edit) {
            (Some((a, _)), _) => a,
            (None, Some(te)) => te.caret.saturating_sub(1),
            _ => 0,
        };
        let color = n
            .style
            .fills
            .last()
            .map_or(Color::INK, |p| p.preview_color());
        Some(n.text_style.char_style(at, *font_size, color))
    }

    pub fn toggle_bold(&mut self) {
        let on = !self.current_char_style().is_some_and(|s| s.bold);
        self.style_text("bold", |s| s.bold = Some(on));
    }
    pub fn toggle_italic(&mut self) {
        let on = !self.current_char_style().is_some_and(|s| s.italic);
        self.style_text("italic", |s| s.italic = Some(on));
    }
    pub fn toggle_underline(&mut self) {
        let on = !self.current_char_style().is_some_and(|s| s.underline);
        self.style_text("font", |s| s.underline = Some(on));
    }
    pub fn toggle_strike(&mut self) {
        let on = !self.current_char_style().is_some_and(|s| s.strike);
        self.style_text("font", |s| s.strike = Some(on));
    }
}

fn byte_at(t: &str, char_idx: usize) -> usize {
    t.char_indices().nth(char_idx).map_or(t.len(), |(b, _)| b)
}
