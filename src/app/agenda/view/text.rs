use crate::agenda::{AgendaResultSnapshot, format_agenda_text};
use gpui::{Div, ParentElement, Styled, div, px};

pub(crate) fn agenda_text(result: &AgendaResultSnapshot) -> Div {
    div()
        .p_4()
        .font_family("Menlo")
        .text_size(px(12.0))
        .child(format_agenda_text(result))
}
