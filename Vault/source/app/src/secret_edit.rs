//! TextEdit receives only a mask: its undo history never owns secrets.
//! Revealing a field affects painting only. Use the vault's Copy actions to copy real values.
use eframe::egui::{self, TextBuffer, text::CharIndex};
use std::{cell::RefCell, ops::Range, rc::Rc};
use zeroize::Zeroizing;

struct MaskedBuffer<'a> {
    value: Rc<RefCell<&'a mut String>>,
    mask: String,
}
impl<'a> MaskedBuffer<'a> {
    fn new(value: &'a mut String) -> Self {
        let mut buffer = Self {
            value: Rc::new(RefCell::new(value)),
            mask: String::new(),
        };
        buffer.refresh();
        buffer
    }
    fn refresh(&mut self) {
        self.mask = self
            .value
            .borrow()
            .chars()
            .map(|c| if c == '\n' { '\n' } else { '•' })
            .collect();
    }
    fn byte(value: &str, index: usize) -> usize {
        value
            .char_indices()
            .nth(index)
            .map_or(value.len(), |(i, _)| i)
    }
}
impl TextBuffer for MaskedBuffer<'_> {
    fn is_mutable(&self) -> bool {
        true
    }
    fn as_str(&self) -> &str {
        &self.mask
    }
    fn insert_text(&mut self, text: &str, index: CharIndex) -> usize {
        {
            let mut value = self.value.borrow_mut();
            // Do not reallocate a plaintext String and leave its old allocation unwiped.
            let old = Zeroizing::new(std::mem::take(&mut **value));
            let at = Self::byte(&old, index.0);
            let mut next = String::with_capacity(old.len() + text.len());
            next.push_str(&old[..at]);
            next.push_str(text);
            next.push_str(&old[at..]);
            **value = next;
        }
        self.refresh();
        text.chars().count()
    }
    fn delete_char_range(&mut self, range: Range<CharIndex>) {
        {
            let mut value = self.value.borrow_mut();
            let old = Zeroizing::new(std::mem::take(&mut **value));
            let start = Self::byte(&old, range.start.0);
            let end = Self::byte(&old, range.end.0).max(start);
            let mut next = String::with_capacity(old.len() - (end - start));
            next.push_str(&old[..start]);
            next.push_str(&old[end..]);
            **value = next;
        }
        self.refresh();
    }
    // egui uses this for undo/redo. A mask must never replace the actual value.
    fn replace_with(&mut self, _text: &str) {}
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<String>()
    }
}

pub fn show(
    ui: &mut egui::Ui,
    value: &mut String,
    reveal: bool,
    multiline: bool,
    width: f32,
) -> egui::Response {
    // Do not copy masks or let Cut silently destroy a value that was never copied.
    // The surrounding vault provides explicit protected Copy actions.
    let id = ui.next_auto_id();
    if ui.memory(|memory| memory.has_focus(id)) {
        ui.input_mut(|input| {
            input
                .events
                .retain(|event| !matches!(event, egui::Event::Copy | egui::Event::Cut))
        });
    }
    let mut buffer = MaskedBuffer::new(value);
    let value = buffer.value.clone();
    let mut layout = |ui: &egui::Ui, mask: &dyn TextBuffer, wrap: f32| {
        let text = if reveal {
            value.borrow().to_string()
        } else {
            mask.as_str().to_owned()
        };
        let font = egui::TextStyle::Body.resolve(ui.style());
        ui.fonts_mut(|fonts| fonts.layout(text, font, ui.visuals().text_color(), wrap))
    };
    let edit = if multiline {
        egui::TextEdit::multiline(&mut buffer).desired_rows(4)
    } else {
        egui::TextEdit::singleline(&mut buffer)
    };
    let mut output = edit
        .password(!reveal)
        .desired_width(width)
        .layouter(&mut layout)
        .show(ui);
    output.state.clear_undoer();
    output.state.store(ui.ctx(), output.response.id);
    output.response.response.on_hover_text("Undo and keyboard copy/cut are disabled for vault fields. Use the vault’s Copy buttons for clipboard access.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_edits_never_expose_plaintext_through_text_buffer() {
        let mut value = Zeroizing::new(String::from("a🔑é\n秘密"));
        let mut b = MaskedBuffer::new(&mut value);
        assert_eq!(b.as_str(), "•••\n••");
        b.delete_char_range(CharIndex(1)..CharIndex(3));
        b.insert_text("🔒z", CharIndex(1));
        assert_eq!(&***b.value.borrow(), "a🔒z\n秘密");
        b.replace_with("stale undo data");
        assert_eq!(&***b.value.borrow(), "a🔒z\n秘密");
        assert!(!b.as_str().contains('秘'));
    }
    #[test]
    fn hidden_and_revealed_fields_leave_no_undo_secret_after_drop() {
        for reveal in [false, true] {
            let ctx = egui::Context::default();
            let mut secret = Zeroizing::new("undo-canary".to_owned());
            let mut id = egui::Id::NULL;
            for _ in 0..3 {
                let mut output = ctx.run_ui(Default::default(), |ui| {
                    id = show(ui, &mut secret, reveal, false, 200.0).id;
                });
                output.textures_delta.clear();
            }
            drop(secret);
            let state = egui::TextEdit::load_state(&ctx, id).unwrap();
            assert!(
                state
                    .undoer()
                    .undo(&(egui::text::CCursorRange::default(), String::new()))
                    .is_none()
            );
        }
    }
    #[test]
    fn masked_edit_neither_copies_masks_nor_loses_values_on_cut() {
        for event in [egui::Event::Copy, egui::Event::Cut] {
            let ctx = egui::Context::default();
            let mut secret = Zeroizing::new("clipboard-canary".to_owned());
            let mut id = egui::Id::NULL;
            let mut output = ctx.run_ui(Default::default(), |ui| {
                let response = show(ui, &mut secret, true, false, 200.0);
                id = response.id;
                response.request_focus();
            });
            output.textures_delta.clear();
            let mut state = egui::TextEdit::load_state(&ctx, id).unwrap();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(secret.chars().count()),
                )));
            state.store(&ctx, id);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![event],
                    ..Default::default()
                },
                |ui| {
                    show(ui, &mut secret, true, false, 200.0);
                },
            );
            assert_eq!(&*secret, "clipboard-canary");
            assert!(
                !output
                    .platform_output
                    .commands
                    .iter()
                    .any(|command| matches!(command, egui::OutputCommand::CopyText(_)))
            );
            output.textures_delta.clear();
        }
    }
}
