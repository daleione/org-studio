use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui::{
    AnyElement, Entity, InteractiveElement, ParentElement, Styled, div, prelude::*, px, rgb,
};

use crate::{app::WorkspaceWindow, theme::current_theme};

const MAX_VISIBLE_SEGMENTS: usize = 5;

#[derive(Clone, Debug, Eq, PartialEq)]
enum BreadcrumbItem {
    Segment {
        label: Arc<str>,
        path: Arc<Path>,
        current: bool,
    },
    Ellipsis {
        path: Arc<Path>,
    },
}

pub(super) fn file_manager_breadcrumb(
    entity: Entity<WorkspaceWindow>,
    directory: &Path,
) -> gpui::Div {
    let items = directory_breadcrumbs(directory, home_directory().as_deref(), MAX_VISIBLE_SEGMENTS);
    div()
        .min_w(px(0.0))
        .flex_1()
        .flex()
        .items_center()
        .overflow_hidden()
        .children(items.into_iter().enumerate().flat_map(|(index, item)| {
            let separator = (index > 0).then(|| {
                div()
                    .mx_1()
                    .flex_none()
                    .text_color(rgb(current_theme().foreground_dim))
                    .text_size(px(11.0))
                    .child("›")
                    .into_any_element()
            });
            separator
                .into_iter()
                .chain(std::iter::once(render_breadcrumb_item(
                    entity.clone(),
                    index,
                    item,
                )))
        }))
}

fn render_breadcrumb_item(
    entity: Entity<WorkspaceWindow>,
    index: usize,
    item: BreadcrumbItem,
) -> AnyElement {
    match item {
        BreadcrumbItem::Ellipsis { path } => div()
            .id(("dired-breadcrumb-ellipsis", index))
            .flex_none()
            .px_1()
            .py(px(3.0))
            .rounded_sm()
            .cursor_pointer()
            .text_color(rgb(current_theme().foreground_dim))
            .text_size(px(12.0))
            .hover(|style| style.bg(rgb(current_theme().background_alt)))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.file_manager.context_menu = None;
                    this.open_file_manager(path.to_path_buf(), cx);
                });
            })
            .child("…")
            .into_any_element(),
        BreadcrumbItem::Segment {
            label,
            path,
            current,
        } => div()
            .id(("dired-breadcrumb", index))
            .min_w(px(0.0))
            .max_w(px(if current { 280.0 } else { 180.0 }))
            .px_1()
            .py(px(3.0))
            .rounded_sm()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .cursor_pointer()
            .text_size(px(12.5))
            .font_weight(if current {
                gpui::FontWeight::SEMIBOLD
            } else {
                gpui::FontWeight::NORMAL
            })
            .text_color(rgb(if current {
                current_theme().foreground
            } else {
                current_theme().foreground_dim
            }))
            .hover(|style| style.bg(rgb(current_theme().background_alt)))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.file_manager.context_menu = None;
                    if this
                        .file_manager
                        .session
                        .as_ref()
                        .is_some_and(|session| session.directory() == path.as_ref())
                    {
                        this.reload_file_manager(cx);
                    } else {
                        this.open_file_manager(path.to_path_buf(), cx);
                    }
                });
            })
            .child(label.to_string())
            .into_any_element(),
    }
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn directory_breadcrumbs(
    directory: &Path,
    home: Option<&Path>,
    max_visible_segments: usize,
) -> Vec<BreadcrumbItem> {
    let segments = if let Some(home) = home.filter(|home| directory.starts_with(home)) {
        let mut segments = vec![segment("~", home, directory)];
        if let Ok(relative) = directory.strip_prefix(home) {
            let mut path = home.to_path_buf();
            for component in relative.components() {
                path.push(component.as_os_str());
                segments.push(segment(
                    component.as_os_str().to_string_lossy(),
                    &path,
                    directory,
                ));
            }
        }
        segments
    } else {
        directory
            .ancestors()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|path| {
                let label = path
                    .file_name()
                    .map(|name| name.to_string_lossy())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| path.as_os_str().to_string_lossy());
                segment(label, path, directory)
            })
            .collect()
    };
    collapse_middle(segments, max_visible_segments)
}

fn segment(label: impl Into<Arc<str>>, path: &Path, directory: &Path) -> BreadcrumbItem {
    BreadcrumbItem::Segment {
        label: label.into(),
        path: Arc::from(path),
        current: path == directory,
    }
}

fn collapse_middle(
    segments: Vec<BreadcrumbItem>,
    max_visible_segments: usize,
) -> Vec<BreadcrumbItem> {
    if segments.len() <= max_visible_segments || max_visible_segments < 3 {
        return segments;
    }
    let trailing_count = max_visible_segments - 2;
    let trailing_start = segments.len() - trailing_count;
    let mut collapsed = Vec::with_capacity(max_visible_segments);
    collapsed.push(segments[0].clone());
    let hidden_destination = match &segments[trailing_start - 1] {
        BreadcrumbItem::Segment { path, .. } => path.clone(),
        BreadcrumbItem::Ellipsis { .. } => unreachable!("input segments are never collapsed"),
    };
    collapsed.push(BreadcrumbItem::Ellipsis {
        path: hidden_destination,
    });
    collapsed.extend_from_slice(&segments[trailing_start..]);
    collapsed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(items: &[BreadcrumbItem]) -> Vec<&str> {
        items
            .iter()
            .map(|item| match item {
                BreadcrumbItem::Segment { label, .. } => label.as_ref(),
                BreadcrumbItem::Ellipsis { .. } => "…",
            })
            .collect()
    }

    #[test]
    fn home_is_compacted_to_tilde_and_every_visible_segment_keeps_its_path() {
        let home = Path::new("/Users/tester");
        let directory = home.join("projects/notes/work/database/postgres");
        let items = directory_breadcrumbs(&directory, Some(home), 5);

        assert_eq!(labels(&items), ["~", "…", "work", "database", "postgres"]);
        assert!(matches!(
            items.last(),
            Some(BreadcrumbItem::Segment { path, current: true, .. })
                if path.as_ref() == directory
        ));
        assert!(matches!(
            &items[1],
            BreadcrumbItem::Ellipsis { path }
                if path.as_ref() == home.join("projects/notes")
        ));
    }

    #[test]
    fn paths_outside_home_keep_the_filesystem_root() {
        let directory = Path::new("/opt/org-studio/docs");
        let items = directory_breadcrumbs(directory, Some(Path::new("/Users/tester")), 5);

        assert_eq!(labels(&items), ["/", "opt", "org-studio", "docs"]);
        assert!(matches!(
            items.first(),
            Some(BreadcrumbItem::Segment { path, .. }) if path.as_ref() == Path::new("/")
        ));
    }

    #[test]
    fn short_paths_are_not_collapsed() {
        let directory = Path::new("/Users/tester/notes");
        let items = directory_breadcrumbs(directory, Some(Path::new("/Users/tester")), 5);
        assert_eq!(labels(&items), ["~", "notes"]);
        assert!(
            !items
                .iter()
                .any(|item| matches!(item, BreadcrumbItem::Ellipsis { .. }))
        );
    }
}
