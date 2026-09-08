use gpui::{Div, Styled, div, px};

/// Shared by the table header and every task row. Fixed cells must never shrink.
#[derive(Clone, Copy)]
pub(crate) struct TaskColumns {
    pub(crate) source: bool,
    pub(crate) plan: bool,
    pub(crate) priority: bool,
    pub(crate) tags: bool,
}

impl TaskColumns {
    pub(crate) const CHECK: f32 = 18.;
    pub(crate) const SOURCE: f32 = 88.;
    pub(crate) const TIME: f32 = 48.;
    pub(crate) const PLAN: f32 = 80.;
    pub(crate) const STATUS: f32 = 64.;
    pub(crate) const PRIORITY: f32 = 32.;
    pub(crate) const TAGS: f32 = 112.;
    pub(crate) const GUTTER: f32 = 24.;
    pub(crate) const GAP: f32 = 8.;

    pub(crate) fn plan_width(language: crate::i18n::Language) -> f32 {
        match language {
            crate::i18n::Language::Chinese => 44.,
            crate::i18n::Language::English => Self::PLAN,
        }
    }

    pub(crate) fn for_width(width: f32, has_tags: bool) -> Self {
        Self {
            source: width >= 480.,
            priority: width >= 600.,
            plan: width >= 720.,
            tags: has_tags && width >= 960.,
        }
    }

    pub(crate) fn row() -> Div {
        div().px(px(12.)).flex().items_center().gap(px(Self::GAP))
    }

    pub(crate) fn cell(width: f32) -> Div {
        div()
            .w(px(width))
            .flex_none()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_columns_reserve_title_space_at_each_breakpoint() {
        for width in [320., 480., 600., 720., 960., 1200.] {
            let columns = TaskColumns::for_width(width, true);
            let mut fixed = vec![TaskColumns::CHECK, TaskColumns::TIME, TaskColumns::STATUS];
            for (visible, size) in [
                (columns.source, TaskColumns::SOURCE),
                (columns.plan, TaskColumns::PLAN),
                (columns.priority, TaskColumns::PRIORITY),
                (columns.tags, TaskColumns::TAGS),
            ] {
                if visible {
                    fixed.push(size);
                }
            }
            let title = width
                - 2. * (TaskColumns::GUTTER + 1. + 12.)
                - fixed.iter().sum::<f32>()
                - fixed.len() as f32 * TaskColumns::GAP;
            assert!(title >= 88., "width={width}, title={title}");
        }
        assert!(!TaskColumns::for_width(1400., false).tags);
    }
}
