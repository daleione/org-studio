use super::{PreviewApp, div, img, px, rgb};
use crate::recent_documents::RecentDocument;
use gpui::{Entity, ExternalPaths, FontWeight, SharedString, prelude::*};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub(in crate::preview) fn render_home(
    entity: Entity<PreviewApp>,
    recent_documents: &[RecentDocument],
    error: Option<&str>,
    opening: Option<&std::path::Path>,
) -> gpui::Div {
    let open_entity = entity.clone();
    let drop_entity = entity.clone();
    let clear_entity = entity.clone();
    div()
        .size_full()
        .bg(rgb(0xf7f7f8))
        .font_family(".SystemUIFont")
        .child(
            div()
                .id("home-scroll")
                .mx_auto()
                .w_full()
                .h_full()
                .max_w(px(656.0))
                .overflow_y_scroll()
                .px_7()
                .pt(px(54.0))
                .pb_12()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .child(
                            div()
                                .w(px(78.0))
                                .h(px(78.0))
                                .rounded(px(18.0))
                                .overflow_hidden()
                                .border_1()
                                .border_color(rgb(0xd8cec4))
                                .bg(rgb(0xeee7df))
                                .shadow_sm()
                                .child(img(home_icon_path()).size_full()),
                        )
                        .child(
                            div()
                                .mt_5()
                                .text_size(px(27.0))
                                .line_height(px(32.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(0x242428))
                                .child("Org Studio"),
                        )
                        .child(
                            div()
                                .mt_2()
                                .text_size(px(14.0))
                                .text_color(rgb(0x7b7b83))
                                .child("Your life in plain text."),
                        )
                        .child(
                            div()
                                .id("home-open-document")
                                .relative()
                                .mt_6()
                                .w_full()
                                .h(px(88.0))
                                .rounded(px(13.0))
                                .border_1()
                                .border_color(rgb(0xe1e1e4))
                                .bg(rgb(0xffffff))
                                .shadow_sm()
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(|style| style.border_color(rgb(0xb9cee5)).bg(rgb(0xfcfdff)))
                                .on_click(move |_, _, cx| {
                                    open_entity.update(cx, |this, cx| this.choose_file(cx));
                                })
                                .on_drop(move |paths: &ExternalPaths, _, cx| {
                                    drop_entity
                                        .update(cx, |this, cx| this.open_dropped_paths(paths, cx));
                                })
                                .child(picker_copy(
                                    opening
                                        .and_then(|path| path.file_name())
                                        .map(|name| format!("Opening {}", name.to_string_lossy()))
                                        .unwrap_or_else(|| "Open Document".to_owned()),
                                    opening
                                        .map(|path| path.display().to_string())
                                        .unwrap_or_else(|| {
                                            "Click to choose a file, or drop one here".to_owned()
                                        }),
                                    0x242428,
                                    0x7b7b83,
                                ))
                                .child(
                                    div()
                                        .invisible()
                                        .absolute()
                                        .inset_0()
                                        .rounded(px(13.0))
                                        .border_2()
                                        .border_color(rgb(0x78a6d6))
                                        .bg(rgb(0xedf5fd))
                                        .drag_over::<ExternalPaths>(|style, _, _, _| {
                                            style.visible()
                                        })
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(picker_copy(
                                            "Drop to Open",
                                            "Release the file to open it",
                                            0x2868a8,
                                            0x6684a3,
                                        )),
                                ),
                        ),
                )
                .when_some(error.map(str::to_owned), |view, error| {
                    view.child(
                        div()
                            .mt_4()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rgb(0xf2c8c2))
                            .bg(rgb(0xfff2f0))
                            .px_4()
                            .py_3()
                            .text_size(px(13.0))
                            .text_color(rgb(0xa12b1f))
                            .child(error),
                    )
                })
                .child(
                    div()
                        .mt(px(42.0))
                        .h(px(32.0))
                        .px_2()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(0x626268))
                                .child("Recent"),
                        )
                        .when(!recent_documents.is_empty(), |view| {
                            view.child(
                                div()
                                    .id("home-clear-recents")
                                    .cursor_pointer()
                                    .px_2()
                                    .py_1()
                                    .rounded(px(5.0))
                                    .text_size(px(12.0))
                                    .text_color(rgb(0xa1a1a8))
                                    .hover(|style| {
                                        style.bg(rgb(0xeeeeef)).text_color(rgb(0x49494f))
                                    })
                                    .on_click(move |_, _, cx| {
                                        clear_entity
                                            .update(cx, |this, cx| this.clear_recent_documents(cx));
                                    })
                                    .child("Clear"),
                            )
                        }),
                )
                .child(render_recents(entity, recent_documents)),
        )
}

pub(in crate::preview) fn render_loading(path: &std::path::Path) -> gpui::Div {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    div()
        .size_full()
        .bg(rgb(0xf7f7f8))
        .font_family(".SystemUIFont")
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .max_w(px(520.0))
                .px_7()
                .flex()
                .flex_col()
                .items_center()
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(0x303034))
                        .child(format!("Opening {name}")),
                )
                .child(
                    div()
                        .mt_2()
                        .text_size(px(12.0))
                        .text_color(rgb(0x83838a))
                        .overflow_hidden()
                        .child(path.display().to_string()),
                ),
        )
}

fn picker_copy(
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    title_color: u32,
    detail_color: u32,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .child(
            div()
                .text_size(px(14.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(title_color))
                .child(title.into()),
        )
        .child(
            div()
                .mt_1()
                .text_size(px(12.0))
                .text_color(rgb(detail_color))
                .child(detail.into()),
        )
}

fn render_recents(entity: Entity<PreviewApp>, documents: &[RecentDocument]) -> gpui::Div {
    let list = div()
        .w_full()
        .rounded(px(13.0))
        .border_1()
        .border_color(rgb(0xe1e1e4))
        .bg(rgb(0xffffff))
        .shadow_sm()
        .overflow_hidden();
    if documents.is_empty() {
        return list.child(
            div()
                .py_6()
                .text_align(gpui::TextAlign::Center)
                .text_size(px(13.0))
                .text_color(rgb(0x8d8d94))
                .child("Documents you open will appear here."),
        );
    }
    documents
        .iter()
        .enumerate()
        .fold(list, |list, (index, document)| {
            let path = document.path.clone();
            let row_entity = entity.clone();
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let parent = abbreviated_parent(&path);
            let kind = path
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_uppercase)
                .unwrap_or_else(|| "DOC".to_owned());
            let kind_color = if kind == "ORG" { 0xa45c3e } else { 0x588bc1 };
            list.child(
                div()
                    .id(("recent-document", index))
                    .h(px(62.0))
                    .px_3()
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap_3()
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(rgb(0xe8e8ea))
                    })
                    .hover(|style| style.bg(rgb(0xf3f3f4)))
                    .on_click(move |_, _, cx| {
                        row_entity.update(cx, |this, cx| this.open_recent(path.clone(), cx));
                    })
                    .child(
                        div()
                            .w(px(30.0))
                            .h(px(35.0))
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(rgb(0xd9dadd))
                            .bg(rgb(0xf6f7f9))
                            .flex()
                            .items_end()
                            .justify_center()
                            .pb_1()
                            .text_size(px(8.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(kind_color))
                            .child(kind),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(0x303034))
                                    .overflow_hidden()
                                    .child(name),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .text_size(px(12.0))
                                    .text_color(rgb(0x83838a))
                                    .overflow_hidden()
                                    .child(parent),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.0))
                            .text_color(rgb(0xa1a1a8))
                            .child(relative_time(document.opened_at)),
                    ),
            )
        })
}

fn home_icon_path() -> PathBuf {
    if let Ok(executable) = std::env::current_exe()
        && let Some(contents) = executable.parent().and_then(|path| path.parent())
    {
        let installed = contents.join("Resources/OrgStudio.png");
        if installed.is_file() {
            return installed;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/macos/OrgStudio.png")
}

fn abbreviated_parent(path: &std::path::Path) -> String {
    let Some(parent) = path.parent() else {
        return String::new();
    };
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(relative) = parent.strip_prefix(home)
    {
        return if relative.as_os_str().is_empty() {
            "~".to_owned()
        } else {
            format!("~/{}", relative.display())
        };
    }
    parent.display().to_string()
}

fn relative_time(opened_at: u64) -> &'static str {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    match now.saturating_sub(opened_at) {
        0..=3_599 => "Just now",
        3_600..=86_399 => "Today",
        86_400..=172_799 => "Yesterday",
        _ => "Earlier",
    }
}
