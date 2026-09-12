use super::labels::{Group, Item};
use crate::app::status_line::shell::ShellShape;
use gpui::{TextRun, Window, font, px, rgb};
use std::ops::Range;

pub(super) const FONT: &str = ".SystemUIFont";
pub(super) const KEY_FONT: &str = "Menlo";
pub(super) const FONT_SIZE: f32 = 12.;
pub(super) const KEY_SIZE: f32 = 11.;
pub(super) const KEY_HEIGHT: f32 = 25.;
pub(super) const KEY_PADDING: f32 = 7.;
pub(super) const ROW_HEIGHT: f32 = 34.;
pub(super) const ROW_PADDING: f32 = 6.;
pub(super) const ITEM_GAP: f32 = 10.;
pub(super) const HEADER_HEIGHT: f32 = 26.;
pub(super) const GAP: f32 = 12.;
pub(super) const PADDING: f32 = 14.;
pub(super) const FOOTER_HEIGHT: f32 = 43.;
const BORDER: f32 = 1.;

pub(super) fn key_label(key: &str) -> &str {
    match key {
        "left" => "←",
        "right" => "→",
        "up" => "↑",
        "down" => "↓",
        _ => key,
    }
}

#[derive(Clone)]
pub(super) struct Column {
    pub group: Option<Group>,
    pub items: Range<usize>,
}
pub(super) struct Row {
    pub columns: Vec<Column>,
    pub height: f32,
}
#[derive(Default)]
pub(super) struct Layout {
    pub width: f32,
    height: f32,
    pub column_width: f32,
    pub key_width: f32,
    pub items: Vec<Item>,
    pub rows: Vec<Row>,
}

fn measure(window: &Window, text: &str, family: &str, size: f32) -> f32 {
    let run = TextRun {
        len: text.len(),
        font: font(family),
        color: rgb(0x53657b).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    f32::from(
        window
            .text_system()
            .shape_line(text.to_owned().into(), px(size), &[run], None)
            .width,
    )
    .ceil()
}

impl Layout {
    pub fn new(
        mut items: Vec<Item>,
        available: f32,
        viewport_height: f32,
        window: &Window,
    ) -> Self {
        let grouped = items.len() > 3 && items.iter().any(|i| i.group != items[0].group);
        if grouped {
            items.sort_by_key(|i| i.group);
        }
        let key_width = items
            .iter()
            .map(|i| measure(window, key_label(&i.key), KEY_FONT, KEY_SIZE) + 2. * KEY_PADDING)
            .fold(48., f32::max);
        let preferred_width = items
            .iter()
            .map(|i| {
                measure(window, &i.title, FONT, FONT_SIZE)
                    + key_width
                    + ITEM_GAP
                    + 2. * ROW_PADDING
                    + if i.prefix { GAP } else { 0. }
            })
            .fold(if grouped { 230. } else { 170. }, f32::max);
        let content_width = (available - 2. * (PADDING + BORDER)).max(1.);
        let capacity =
            (((content_width + GAP) / (preferred_width + GAP)).floor() as usize).clamp(1, 3);
        let mut columns = Vec::new();
        if grouped {
            let mut start = 0;
            while start < items.len() {
                let end = start
                    + items[start..]
                        .iter()
                        .take_while(|i| i.group == items[start].group)
                        .count();
                columns.push(Column {
                    group: Some(items[start].group),
                    items: start..end,
                });
                start = end;
            }
        } else {
            let rows = items.len().div_ceil(capacity).max(1);
            for start in (0..items.len()).step_by(rows) {
                columns.push(Column {
                    group: None,
                    items: start..(start + rows).min(items.len()),
                });
            }
        }
        let count = columns.len().clamp(1, capacity);
        let column_width =
            preferred_width.min((content_width - GAP * (count - 1) as f32) / count as f32);
        let width =
            (2. * (PADDING + BORDER) + count as f32 * column_width + GAP * (count - 1) as f32)
                .min(available);
        let rows = columns
            .chunks(count)
            .map(|columns| Row {
                height: columns.iter().map(|c| c.items.len()).max().unwrap_or(0) as f32
                    * ROW_HEIGHT
                    + if grouped { HEADER_HEIGHT } else { 0. },
                columns: columns.to_vec(),
            })
            .collect::<Vec<_>>();
        let body_height =
            rows.iter().map(|r| r.height).sum::<f32>() + GAP * rows.len().saturating_sub(1) as f32;
        let height = (body_height + 2. * (PADDING + BORDER) + FOOTER_HEIGHT)
            .min((viewport_height - 100.).clamp(110., 420.));
        Self {
            width,
            height,
            column_width,
            key_width,
            items,
            rows,
        }
    }
    pub fn shape(&self) -> ShellShape {
        ShellShape {
            width: self.width,
            height: self.height,
            expansion_height: 0.,
            content_opacity: 1.,
        }
    }
}
