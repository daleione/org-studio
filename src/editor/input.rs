use std::ops::Range;

use gpui::{
    Bounds, ClipboardItem, Context, EntityInputHandler, Pixels, UTF16Selection, Window, point, size,
};

use crate::document::{
    ByteOffset, ByteRange, EditOrigin, EditTransaction, Selection, TextEdit, TextSnapshot,
    Utf16Offset,
};

use super::{Composition, LINE_HEIGHT, PlatformRange, SourceEditor};

impl SourceEditor {
    fn byte_range_from_utf16(&self, range: Range<usize>, cx: &gpui::App) -> Option<ByteRange> {
        let snapshot = self.snapshot(cx);
        Some(ByteRange {
            start: snapshot
                .utf16_to_byte(Utf16Offset(range.start as u64))
                .ok()?,
            end: snapshot.utf16_to_byte(Utf16Offset(range.end as u64)).ok()?,
        })
    }

    fn utf16_range_for_bytes(&self, range: ByteRange, cx: &gpui::App) -> Option<Range<usize>> {
        let snapshot = self.snapshot(cx);
        let start = usize::try_from(snapshot.byte_to_utf16(range.start).ok()?.0).ok()?;
        let end = usize::try_from(snapshot.byte_to_utf16(range.end).ok()?.0).ok()?;
        Some(start..end)
    }

    fn apply_composition_update(
        &mut self,
        range: ByteRange,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Option<crate::document::Revision> {
        let revision = self.session.read(cx).revision();
        self.session
            .update(cx, |session, cx| {
                session.apply_transient_edit(
                    EditTransaction::new(revision, vec![TextEdit::new(range, text.to_owned())]),
                    cx,
                )
            })
            .ok()
            .map(|delta| delta.after)
    }

    fn begin_or_update_composition(
        &mut self,
        range: ByteRange,
        range_utf16: Range<usize>,
        text: &str,
        selected_utf16: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        if self.composition.is_none() {
            let snapshot = self.snapshot(cx);
            self.composition = Some(Composition {
                original_range: range,
                original_text: snapshot.copy_range(range),
                before: self.selection,
                revision: self.session.read(cx).revision(),
            });
        }
        let Some(revision) = self.apply_composition_update(range, text, cx) else {
            return;
        };
        self.composition
            .as_mut()
            .expect("composition exists")
            .revision = revision;
        let marked = ByteRange::new(range.start.0, range.start.0 + text.len() as u64);
        let marked_end_utf16 = range_utf16.start + text.encode_utf16().count();
        self.marked = (!text.is_empty()).then_some(PlatformRange {
            bytes: marked,
            utf16: range_utf16.start..marked_end_utf16,
        });
        let selected = selected_utf16.and_then(|selected| {
            let start = utf16_to_byte_in_str(text, selected.start)?;
            let end = utf16_to_byte_in_str(text, selected.end)?;
            Some((
                Selection::new(
                    ByteOffset(range.start.0 + start as u64),
                    ByteOffset(range.start.0 + end as u64),
                ),
                range_utf16.start + selected.start..range_utf16.start + selected.end,
            ))
        });
        if let Some((selection, selection_utf16)) = selected {
            self.selection = selection;
            self.selection_utf16 = selection_utf16;
        } else {
            self.selection = Selection::caret(marked.end);
            self.selection_utf16 = marked_end_utf16..marked_end_utf16;
        }
        self.selection_utf16_reversed = false;
        let snapshot = self.snapshot(cx);
        self.reveal_caret(&snapshot);
        cx.notify();
    }

    pub(super) fn finish_composition(&mut self, cx: &mut Context<Self>) {
        let Some(composition) = self.composition.take() else {
            self.marked = None;
            return;
        };
        let current = self
            .marked
            .take()
            .map_or_else(|| self.selection.range(), |range| range.bytes);
        let snapshot = self.snapshot(cx);
        let current_text = snapshot.copy_range(current);
        let result = self.session.update(cx, |session, _| {
            session.finalize_transient_edit(
                composition.revision,
                vec![TextEdit::new(composition.original_range, current_text)],
                vec![TextEdit::new(current, composition.original_text)],
                composition.before,
                self.selection,
                EditOrigin::Ime,
            )
        });
        debug_assert!(result.is_ok(), "composition revision must remain current");
        cx.notify();
    }
}

impl EntityInputHandler for SourceEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.byte_range_from_utf16(range_utf16, cx)?;
        adjusted_range.replace(self.utf16_range_for_bytes(range, cx)?);
        Some(self.snapshot(cx).copy_range(range))
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.selection_utf16.clone(),
            reversed: self.selection_utf16_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| range.utf16.clone())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.finish_composition(cx);
    }

    fn paste(&mut self, item: ClipboardItem, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            self.replace_selection(&text, EditOrigin::Paste, cx);
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (range, target_utf16) = if let Some(range_utf16) = range_utf16 {
            let Some(range) = self.byte_range_from_utf16(range_utf16.clone(), cx) else {
                return;
            };
            (range, range_utf16)
        } else if let Some(marked) = self.marked.as_ref() {
            (marked.bytes, marked.utf16.clone())
        } else {
            (self.selection.range(), self.selection_utf16.clone())
        };
        if let Some(composition) = self.composition.take() {
            if let Some(revision) = self.apply_composition_update(range, text, cx) {
                let current = ByteRange::new(range.start.0, range.start.0 + text.len() as u64);
                self.selection = Selection::caret(current.end);
                self.marked = None;
                let caret_utf16 = target_utf16.start + text.encode_utf16().count();
                self.selection_utf16 = caret_utf16..caret_utf16;
                self.selection_utf16_reversed = false;
                let result = self.session.update(cx, |session, _| {
                    session.finalize_transient_edit(
                        revision,
                        vec![TextEdit::new(composition.original_range, text.to_owned())],
                        vec![TextEdit::new(current, composition.original_text)],
                        composition.before,
                        self.selection,
                        EditOrigin::Ime,
                    )
                });
                debug_assert!(result.is_ok(), "composition revision must remain current");
                let snapshot = self.snapshot(cx);
                self.reveal_caret(&snapshot);
                cx.notify();
            }
        } else {
            self.selection = Selection::new(range.start, range.end);
            self.selection_utf16 = target_utf16;
            self.selection_utf16_reversed = false;
            self.replace_selection(text, EditOrigin::Typing, cx);
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        selected_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (range, target_utf16) = if let Some(range_utf16) = range_utf16 {
            let Some(range) = self.byte_range_from_utf16(range_utf16.clone(), cx) else {
                return;
            };
            (range, range_utf16)
        } else if let Some(marked) = self.marked.as_ref() {
            (marked.bytes, marked.utf16.clone())
        } else {
            (self.selection.range(), self.selection_utf16.clone())
        };
        self.begin_or_update_composition(range, target_utf16, text, selected_utf16, cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.byte_range_from_utf16(range_utf16, cx)?;
        let row = self
            .hit_rows
            .iter()
            .find(|row| range.start >= row.range.start && range.start <= row.range.end)?;
        let start = (range.start.0 - row.range.start.0).min(row.layout.len() as u64) as usize;
        let end = range
            .end
            .0
            .saturating_sub(row.range.start.0)
            .min(row.layout.len() as u64) as usize;
        Some(Bounds::new(
            point(
                row.text_origin_x + row.layout.x_for_index(start),
                row.origin_y,
            ),
            size(
                row.layout.x_for_index(end) - row.layout.x_for_index(start),
                gpui::px(LINE_HEIGHT),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let byte = self.hit_test(point);
        usize::try_from(self.snapshot(cx).byte_to_utf16(byte).ok()?.0).ok()
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_composition(cx);
        if let Some(range) = self.byte_range_from_utf16(range_utf16.clone(), cx) {
            self.selection = Selection::new(range.start, range.end);
            self.selection_utf16 = range_utf16;
            self.selection_utf16_reversed = false;
            let snapshot = self.snapshot(cx);
            self.reveal_caret(&snapshot);
            cx.notify();
        }
    }
}

fn utf16_to_byte_in_str(text: &str, target: usize) -> Option<usize> {
    let mut utf16 = 0;
    let mut bytes = 0;
    for character in text.chars() {
        if utf16 == target {
            return Some(bytes);
        }
        let width = character.len_utf16();
        if utf16 + width > target {
            return None;
        }
        utf16 += width;
        bytes += character.len_utf8();
    }
    (utf16 == target).then_some(bytes)
}
