//! One menu model and renderer for toolbar menus, context actions, and sorting.
use std::{cell::Cell, path::Path, rc::Rc, sync::Arc, time::Instant};

use gpui::{
    App, Bounds, Context, Entity, FocusHandle, KeyDownEvent, MouseButton, Pixels, Point,
    ScrollHandle, Size, TextRun, Window, anchored, canvas, deferred, div, point, prelude::*, px,
    rgb, size,
};

use super::WorkspaceWindow;
use crate::{
    command::{FileTreeCommand, file_tree_command},
    components::{
        UiTextExt,
        icon::{Icon, icon},
    },
    typography::{FontRole, cjk_runs, ui_font, ui_px},
};

const ROW_HEIGHT: f32 = 26.;
const SEPARATOR_HEIGHT: f32 = 9.;
const FRAME: f32 = 6.;
const MARGIN: f32 = 8.;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum MenuKind {
    Context,
    New,
    More,
}

impl MenuKind {
    pub(super) fn toolbar_index(self) -> Option<usize> {
        let command = match self {
            Self::New => FileTreeCommand::NewMenu,
            Self::More => FileTreeCommand::MoreMenu,
            Self::Context => return None,
        };
        super::TOOLBAR_COMMANDS
            .iter()
            .position(|candidate| *candidate == command)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Main,
    Sort,
}

struct Panel {
    items: Vec<MenuItem>,
    metrics: MenuMetrics,
    selected: Option<usize>,
    scroll: ScrollHandle,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    query: String,
    typed_at: Instant,
}

impl Panel {
    fn new(items: Vec<MenuItem>, window: &Window) -> Self {
        Self {
            metrics: MenuMetrics::new(&items, window),
            items,
            selected: None,
            scroll: ScrollHandle::new(),
            bounds: Default::default(),
            query: String::new(),
            typed_at: Instant::now(),
        }
    }

    fn reset(&mut self, keyboard: bool) {
        self.selected = keyboard
            .then(|| self.items.iter().position(|item| item.enabled))
            .flatten();
        self.scroll.set_offset(Point::default());
        self.query.clear();
    }

    fn key_down(&mut self, event: &KeyDownEvent) {
        let key = event.keystroke.key.as_str();
        let next = match key {
            "up" | "down" | "home" | "end" => {
                self.query.clear();
                self.navigate(key)
            }
            _ if key.chars().count() == 1 => {
                self.type_ahead(event.keystroke.key_char.as_deref().unwrap_or(key))
            }
            _ => None,
        };
        if let Some(index) = next {
            self.selected = Some(index);
            self.scroll.scroll_to_item(index);
        }
    }

    fn navigate(&self, key: &str) -> Option<usize> {
        let count = self.items.len();
        if count == 0 {
            return None;
        }
        let reverse = matches!(key, "up" | "end");
        let start = match key {
            "home" => 0,
            "end" => count - 1,
            "up" => (self.selected.unwrap_or(0) + count - 1) % count,
            _ => self.selected.map_or(0, |index| (index + 1) % count),
        };
        (0..count)
            .map(|offset| {
                if reverse {
                    (start + count - offset) % count
                } else {
                    (start + offset) % count
                }
            })
            .find(|index| self.items[*index].enabled)
    }

    fn type_ahead(&mut self, typed: &str) -> Option<usize> {
        let now = Instant::now();
        if now.duration_since(self.typed_at).as_millis() > 750 {
            self.query.clear();
        }
        self.typed_at = now;
        let typed = typed.to_lowercase();
        if self.query != typed {
            self.query.push_str(&typed);
        }
        let count = self.items.len();
        let start = self.selected.map_or(0, |index| (index + 1) % count);
        (0..count)
            .map(|offset| (start + offset) % count)
            .find(|index| {
                self.items[*index].enabled
                    && self.items[*index]
                        .label
                        .to_lowercase()
                        .starts_with(&self.query)
            })
    }
}

pub(in crate::app::file_manager) struct TreeMenu {
    pub(super) kind: MenuKind,
    pointer: Option<Point<Pixels>>,
    main: Panel,
    sort: Panel,
    active: Page,
    sort_anchor: Rc<Cell<Bounds<Pixels>>>,
    pub(in crate::app::file_manager) targets: Arc<[Arc<Path>]>,
    pub(super) creation_directory: Option<std::path::PathBuf>,
    focus: FocusHandle,
}

impl TreeMenu {
    fn panel(&self, page: Page) -> &Panel {
        match page {
            Page::Main => &self.main,
            Page::Sort => &self.sort,
        }
    }
    fn panel_mut(&mut self, page: Page) -> &mut Panel {
        match page {
            Page::Main => &mut self.main,
            Page::Sort => &mut self.sort,
        }
    }
    fn contains(&self, position: Point<Pixels>) -> bool {
        self.main.bounds.get().contains(&position)
            || (self.active == Page::Sort && self.sort.bounds.get().contains(&position))
    }

    fn hover(&mut self, page: Page, index: usize) {
        if page == Page::Sort && self.active != page {
            return;
        }
        let panel = self.panel_mut(page);
        let Some(item) = panel.items.get(index) else {
            return;
        };
        panel.selected = item.enabled.then_some(index);
        if page == Page::Main {
            let next = if item.enabled && item.action == MenuAction::Sort {
                Page::Sort
            } else {
                Page::Main
            };
            if next == Page::Sort && self.active != next {
                self.sort.reset(false);
            }
            self.active = next;
        }
    }

    fn activate(&mut self, page: Page, index: usize, keyboard: bool) -> Option<FileTreeCommand> {
        if page == Page::Sort && self.active != page {
            return None;
        }
        let item = self
            .panel(page)
            .items
            .get(index)
            .filter(|item| item.enabled)?;
        match item.action {
            MenuAction::Command(command) => Some(command),
            MenuAction::Sort => {
                self.sort.reset(keyboard);
                self.active = Page::Sort;
                None
            }
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent) -> Option<FileTreeCommand> {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            let stroke = crate::keymap::KeyStroke::new(
                key,
                modifiers.control,
                modifiers.alt,
                modifiers.shift,
                modifiers.platform,
            );
            return self
                .main
                .items
                .iter()
                .chain(&self.sort.items)
                .find_map(|item| {
                    let MenuAction::Command(command) = item.action else {
                        return None;
                    };
                    (item.enabled
                        && file_tree_command(command).keys.iter().any(|key| {
                            crate::keymap::KeyStroke::parse(key).ok().as_ref() == Some(&stroke)
                        }))
                    .then_some(command)
                });
        }
        let page = self.active;
        match key {
            "right" | "enter" | "space" => {
                if let Some(index) = self.panel(page).selected
                    && (key != "right" || self.panel(page).items[index].action == MenuAction::Sort)
                {
                    return self.activate(page, index, true);
                }
            }
            "left" => self.active = Page::Main,
            _ => self.panel_mut(page).key_down(event),
        }
        None
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum MenuAction {
    Command(FileTreeCommand),
    Sort,
}

struct MenuItem {
    action: MenuAction,
    label: &'static str,
    shortcut: gpui::SharedString,
    enabled: bool,
    checked: bool,
    separator: bool,
}

/// Measurements use the same fonts, rows, separators and padding as the renderer.
struct MenuMetrics {
    size: Size<Pixels>,
    shortcut_width: Pixels,
}

impl MenuMetrics {
    fn new(items: &[MenuItem], window: &Window) -> Self {
        let measure = |text: &str, symbols: bool| {
            let run = TextRun {
                len: text.len(),
                font: if symbols {
                    gpui::font(".SystemUIFont")
                } else {
                    ui_font("system-ui")
                },
                ..Default::default()
            };
            let runs = if symbols {
                vec![run]
            } else {
                cjk_runs(text, vec![run], FontRole::Ui)
            };
            window
                .text_system()
                .shape_line(text.to_owned().into(), ui_px(13.), &runs, None)
                .width
        };
        let label = items
            .iter()
            .map(|item| measure(item.label, false))
            .max()
            .unwrap_or_default();
        let shortcut_width = items
            .iter()
            .map(|item| {
                if item.action == MenuAction::Sort {
                    ui_px(12.)
                } else {
                    measure(&item.shortcut, true)
                }
            })
            .max()
            .unwrap_or_default();
        Self {
            size: size(
                (ui_px(2. * FRAME + 16. + 14. + 6. + 24.) + label + shortcut_width)
                    .max(ui_px(216.))
                    .ceil(),
                ui_px(
                    2. * FRAME
                        + items.len() as f32 * ROW_HEIGHT
                        + items.iter().filter(|item| item.separator).count() as f32
                            * SEPARATOR_HEIGHT,
                ),
            ),
            shortcut_width,
        }
    }
}

fn viewport_limits(viewport: Size<Pixels>) -> Bounds<Pixels> {
    let margin = ui_px(MARGIN)
        .min(viewport.width / 4.)
        .min(viewport.height / 4.);
    Bounds::new(
        point(margin, margin),
        size(viewport.width - margin * 2., viewport.height - margin * 2.),
    )
}

fn popup_bounds(
    desired: Size<Pixels>,
    anchor: Bounds<Pixels>,
    viewport: Size<Pixels>,
    toolbar: bool,
) -> Bounds<Pixels> {
    let limits = viewport_limits(viewport);
    let width = desired.width.min(limits.size.width);
    let gap = if toolbar { ui_px(4.) } else { px(0.) };
    let below = (limits.bottom() - anchor.bottom() - gap).max(px(0.));
    let above = (anchor.top() - gap - limits.top()).max(px(0.));
    let opens_below = desired.height <= below || below >= above;
    let height = desired
        .height
        .min(if opens_below { below } else { above })
        .max(px(1.));
    let x = if toolbar {
        anchor.right() - width
    } else {
        anchor.left()
    };
    let y = if opens_below {
        anchor.bottom() + gap
    } else {
        anchor.top() - gap - height
    };
    Bounds::new(
        point(
            x.clamp(limits.left(), limits.right() - width),
            y.clamp(limits.top(), (limits.bottom() - height).max(limits.top())),
        ),
        size(width, height),
    )
}

fn submenu_bounds(
    desired: Size<Pixels>,
    parent: Bounds<Pixels>,
    row: Bounds<Pixels>,
    viewport: Size<Pixels>,
) -> Bounds<Pixels> {
    let limits = viewport_limits(viewport);
    let size = size(
        desired.width.min(limits.size.width),
        desired.height.min(limits.size.height),
    );
    let right = parent.right() - ui_px(3.);
    let left = parent.left() - size.width + ui_px(3.);
    let x = if right + size.width <= limits.right() {
        right
    } else {
        left
    };
    Bounds::new(
        point(
            x.clamp(limits.left(), limits.right() - size.width),
            (row.top() - ui_px(FRAME)).clamp(limits.top(), limits.bottom() - size.height),
        ),
        size,
    )
}

impl WorkspaceWindow {
    fn file_tree_menu_items(
        &self,
        kind: MenuKind,
        page: Page,
        targets: &[Arc<Path>],
        cx: &App,
    ) -> Vec<MenuItem> {
        use FileTreeCommand as C;
        use MenuAction::{Command as A, Sort};
        let tree = &self.file_manager.tree.model;
        let entries: &[(MenuAction, bool)] = match (kind, page) {
            (_, Page::Sort) => &[
                (A(C::SortName), false),
                (A(C::SortModified), false),
                (A(C::ReverseSort), true),
                (A(C::FoldersFirst), false),
            ],
            (MenuKind::New, _) => &[(A(C::NewFile), false), (A(C::NewFolder), false)],
            (MenuKind::Context, _) => &[
                (A(C::Open), false),
                (A(C::NewFile), true),
                (A(C::NewFolder), false),
                (A(C::Rename), true),
                (A(C::Copy), false),
                (A(C::Move), false),
                (A(C::Trash), false),
                (A(C::RevealInFinder), true),
                (A(C::Refresh), false),
            ],
            (MenuKind::More, _) => &[
                (A(C::Reveal), false),
                (A(C::CollapseAll), false),
                (A(C::Refresh), false),
                (A(C::Hidden), true),
                (A(C::AutoReveal), false),
                (Sort, true),
                (A(C::ChangeRoot), true),
                (A(C::ParentRoot), false),
                (A(C::EnterRoot), false),
            ],
        };
        let root_selected = tree
            .root()
            .is_some_and(|root| targets.iter().any(|path| path.as_ref() == root));
        let can_enter = self
            .file_manager
            .tree
            .directory_root_target(Some(targets))
            .is_some();
        entries
            .iter()
            .copied()
            .chain(
                (kind == MenuKind::Context && page == Page::Main && can_enter)
                    .then_some((A(C::EnterRoot), true)),
            )
            .chain(
                (kind == MenuKind::Context && page == Page::Main && root_selected)
                    .then_some([(A(C::ChangeRoot), true), (A(C::ParentRoot), false)])
                    .into_iter()
                    .flatten(),
            )
            .map(|(action, separator)| {
                let MenuAction::Command(command) = action else {
                    return MenuItem {
                        action,
                        label: self.language.text("files.sort"),
                        shortcut: "".into(),
                        enabled: tree.root().is_some(),
                        checked: false,
                        separator,
                    };
                };
                let enabled = match command {
                    C::Rename => targets.len() == 1 && !root_selected,
                    C::Move | C::Trash => !targets.is_empty() && !root_selected,
                    C::Open => targets.len() == 1,
                    C::Copy | C::RevealInFinder => !targets.is_empty(),
                    C::Reveal => self
                        .document_path(cx)
                        .is_some_and(|path| !path.as_os_str().is_empty()),
                    C::ParentRoot => tree.parent_root().is_some(),
                    C::EnterRoot => can_enter,
                    C::ChangeRoot => true,
                    _ => tree.root().is_some(),
                };
                let label_key = if command == C::Reveal
                    && self
                        .document_path(cx)
                        .is_some_and(|path| !tree.root().is_some_and(|root| path.starts_with(root)))
                {
                    "files.reveal_changes_root"
                } else if command == C::Open
                    && tree.current().is_some_and(|entry| {
                        !entry.is_directory()
                            && !crate::preview::supports_document_path(&entry.path)
                            && !crate::preview::supports_image_path(&entry.path)
                    })
                {
                    "files.open_default"
                } else {
                    file_tree_command(command).label
                };
                let checked = match command {
                    C::Hidden => tree.show_hidden,
                    C::AutoReveal => tree.auto_reveal,
                    C::FoldersFirst => tree.sort.directories_first,
                    C::ReverseSort => {
                        tree.sort.direction == crate::file_manager::SortDirection::Descending
                    }
                    C::SortName => tree.sort.key == crate::file_manager::SortKey::Name,
                    C::SortModified => tree.sort.key == crate::file_manager::SortKey::Modified,
                    _ => false,
                };
                MenuItem {
                    action,
                    label: self.language.text(label_key),
                    enabled,
                    checked,
                    separator,
                    shortcut: file_tree_command(command).shortcut_label().into(),
                }
            })
            .collect()
    }

    pub(super) fn open_file_tree_menu(
        &mut self,
        kind: MenuKind,
        pointer: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tree = &self.file_manager.tree;
        if kind != MenuKind::Context && tree.menu.as_ref().is_some_and(|menu| menu.kind == kind) {
            self.close_file_tree_menu(window, cx);
            return;
        }
        self.focus_sidebar(cx);
        let tree = &mut self.file_manager.tree;
        tree.focus_requested = false;
        // Menu actions share the context captured at opening. Preference commands
        // close the menu, so labels, availability and measurements resolve once.
        let targets = tree.model.operation_targets();
        let creation_directory = tree.model.creation_directory();
        let main_items = self.file_tree_menu_items(kind, Page::Main, &targets, cx);
        let sort_items = if kind == MenuKind::More {
            self.file_tree_menu_items(kind, Page::Sort, &targets, cx)
        } else {
            Vec::new()
        };
        let mut menu = TreeMenu {
            kind,
            pointer,
            main: Panel::new(main_items, window),
            sort: Panel::new(sort_items, window),
            active: Page::Main,
            sort_anchor: Default::default(),
            targets,
            creation_directory,
            focus: cx.focus_handle(),
        };
        menu.main.reset(pointer.is_none());
        window.focus(&menu.focus, cx);
        self.file_manager.tree.menu = Some(menu);
        cx.notify();
    }

    fn close_file_tree_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tree = &mut self.file_manager.tree;
        if let Some(menu) = tree.menu.take() {
            tree.focus_requested = false;
            let focus = menu
                .kind
                .toolbar_index()
                .and_then(|index| tree.toolbar.as_ref().map(|toolbar| toolbar[index].clone()))
                .or_else(|| {
                    tree.input
                        .as_ref()
                        .map(|input| input.read(cx).focus.clone())
                });
            if let Some(focus) = focus {
                window.focus(&focus, cx);
            }
        }
        cx.notify();
    }

    fn dismiss_file_tree_menu_at(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let tree = &mut self.file_manager.tree;
        if tree
            .menu
            .as_ref()
            .is_some_and(|menu| menu.contains(position))
            || tree
                .toolbar_bounds
                .get()
                .iter()
                .any(|bounds| bounds.contains(&position))
        {
            return;
        }
        tree.menu = None;
        if tree.bounds.get().contains(&position) {
            self.focus_sidebar(cx);
        } else {
            self.focus_document(cx);
        }
        cx.notify();
    }

    fn activate_file_tree_menu_item(
        &mut self,
        page: Page,
        index: usize,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(command) = self
            .file_manager
            .tree
            .menu
            .as_mut()
            .and_then(|menu| menu.activate(page, index, keyboard))
        {
            self.dispatch_file_tree_command(command, window, cx);
        }
        cx.notify();
    }

    pub(super) fn file_tree_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = &mut self.file_manager.tree.menu else {
            return;
        };
        let key = event.keystroke.key.as_str();
        let cancel = key == "escape" || (event.keystroke.modifiers.control && key == "g");
        if cancel && menu.active == Page::Sort {
            menu.active = Page::Main;
        } else if cancel || key == "tab" {
            self.close_file_tree_menu(window, cx);
            self.file_manager.tree.focus_requested = key == "tab";
        } else if let Some(command) = menu.key_down(event) {
            self.dispatch_file_tree_command(command, window, cx);
        }
        cx.notify();
    }

    pub(in crate::app::file_manager) fn render_file_tree_menu(
        &self,
        entity: Entity<Self>,
        menu: &TreeMenu,
        window: &Window,
    ) -> gpui::AnyElement {
        let tree = &self.file_manager.tree;
        let anchor = if let Some(index) = menu.kind.toolbar_index() {
            tree.toolbar_bounds.get()[index]
        } else if let Some(pointer) = menu.pointer {
            Bounds::new(pointer, Size::default())
        } else {
            tree.model
                .cursor()
                .and_then(|id| tree.row_bounds.borrow().get(&id).copied())
                .map(|row| {
                    Bounds::new(
                        point(row.left() + ui_px(24.), row.bottom()),
                        Size::default(),
                    )
                })
                .unwrap_or_else(|| Bounds::new(tree.bounds.get().origin, Size::default()))
        };
        let bounds = popup_bounds(
            menu.main.metrics.size,
            anchor,
            window.viewport_size(),
            menu.kind.toolbar_index().is_some(),
        );
        let main = menu.render_panel(entity.clone(), Page::Main, bounds, window);
        let mut overlays = div().child(main);
        if menu.active == Page::Sort {
            let bounds = submenu_bounds(
                menu.sort.metrics.size,
                bounds,
                menu.sort_anchor.get(),
                window.viewport_size(),
            );
            overlays = overlays.child(menu.render_panel(entity, Page::Sort, bounds, window));
        }
        overlays.into_any_element()
    }
}

impl TreeMenu {
    fn render_panel(
        &self,
        entity: Entity<WorkspaceWindow>,
        page: Page,
        bounds: Bounds<Pixels>,
        window: &Window,
    ) -> gpui::AnyElement {
        let colors = crate::theme::navigation_colors();
        let theme = crate::theme::current_theme();
        let panel = self.panel(page);
        let panel_bounds = panel.bounds.clone();
        let dismiss_entity = entity.clone();
        let content = div()
            .id(if page == Page::Main {
                "file-tree-menu"
            } else {
                "file-tree-sort-menu"
            })
            .debug_selector(move || {
                if page == Page::Main {
                    "file-tree-menu".into()
                } else {
                    "file-tree-sort-menu".into()
                }
            })
            .when(page == self.active, |panel| panel.track_focus(&self.focus))
            .relative()
            .occlude()
            .w(bounds.size.width)
            .h(bounds.size.height)
            .p(ui_px(FRAME - 1.))
            .rounded_lg()
            .border_1()
            .border_color(rgb(theme.border))
            .bg(rgb(theme.floating_background >> 8))
            .shadow_lg()
            .font(ui_font("system-ui"))
            .text_size(ui_px(13.))
            .line_height(ui_px(18.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(move |event, _, cx| {
                dismiss_entity.update(cx, |this, cx| {
                    this.dismiss_file_tree_menu_at(event.position, cx)
                });
            })
            .child(
                div()
                    .id(("file-tree-menu-scroll", page as usize))
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&panel.scroll)
                    .children(
                        (0..panel.items.len())
                            .map(|index| self.render_item(&entity, page, index, colors, window)),
                    ),
            )
            .child(
                canvas(
                    move |bounds, _, _| {
                        panel_bounds.set(bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        deferred(
            anchored()
                .position(bounds.origin)
                .snap_to_window_with_margin(ui_px(MARGIN))
                .child(content),
        )
        .with_priority(2)
        .into_any_element()
    }
    fn render_item(
        &self,
        entity: &Entity<WorkspaceWindow>,
        page: Page,
        index: usize,
        colors: crate::theme::NavigationColors,
        window: &Window,
    ) -> gpui::AnyElement {
        let panel = self.panel(page);
        let item = &panel.items[index];
        let theme = crate::theme::current_theme();
        let click_entity = entity.clone();
        let hover_entity = entity.clone();
        let selected = panel.selected == Some(index) && item.enabled;
        let foreground = if !item.enabled {
            theme.foreground_disabled
        } else if selected {
            colors.selected_foreground
        } else {
            theme.foreground
        };
        let shortcut = item.shortcut.clone();
        let sort_anchor = self.sort_anchor.clone();
        let geometry_entity = entity.clone();
        let sort_open = self.active == Page::Sort;
        let mut row = div()
            .id(("file-tree-menu-item", index))
            .debug_selector(move || format!("file-tree-menu-{}-{index}", page as usize))
            .relative()
            .w_full()
            .h(ui_px(ROW_HEIGHT))
            .px(ui_px(8.))
            .rounded_md()
            .flex()
            .items_center()
            .gap(ui_px(6.))
            .text_color(rgb(foreground))
            .when(selected, |row| row.bg(rgb(colors.selected)))
            .on_hover(move |hovered, _, cx| {
                if *hovered {
                    hover_entity.update(cx, |this, cx| {
                        if let Some(menu) = &mut this.file_manager.tree.menu {
                            menu.hover(page, index);
                            cx.notify();
                        }
                    });
                }
            })
            .child(
                div()
                    .id(("file-tree-menu-check", index))
                    .w(ui_px(14.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(item.checked, |mark| {
                        mark.child(icon(Icon::Check, ui_px(11.), foreground, window))
                    }),
            )
            .child(
                div()
                    .id(("file-tree-menu-label", index))
                    .debug_selector(move || {
                        format!("file-tree-menu-label-{}-{index}", page as usize)
                    })
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child_text(item.label),
            )
            .child(
                div()
                    .id(("file-tree-menu-shortcut", index))
                    .debug_selector(move || {
                        format!("file-tree-menu-shortcut-{}-{index}", page as usize)
                    })
                    .w(panel.metrics.shortcut_width)
                    .ml(ui_px(18.))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .font(gpui::font(".SystemUIFont"))
                    .text_color(rgb(if selected {
                        foreground
                    } else if item.enabled {
                        colors.secondary
                    } else {
                        theme.foreground_disabled
                    }))
                    .child(if item.action == MenuAction::Sort {
                        icon(Icon::ChevronRight, ui_px(10.), foreground, window)
                    } else {
                        shortcut.into_any_element()
                    }),
            )
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                click_entity.update(cx, |this, cx| {
                    this.activate_file_tree_menu_item(page, index, false, window, cx)
                });
            });
        if item.action == MenuAction::Sort {
            row = row.child(
                canvas(
                    move |bounds, _, cx| {
                        if sort_anchor.replace(bounds) != bounds && sort_open {
                            cx.notify(geometry_entity.entity_id());
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        }
        div()
            .flex_none()
            .when(item.separator, |wrapper| {
                wrapper.child(
                    div()
                        .h(ui_px(SEPARATOR_HEIGHT))
                        .flex()
                        .items_center()
                        .px(ui_px(8.))
                        .child(div().w_full().h(px(1.)).bg(rgb(theme.border))),
                )
            })
            .child(row)
            .into_any_element()
    }
}
