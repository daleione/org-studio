use crate::typography::{FontRole, role_text};
use gpui::{ParentElement, SharedString};

/// Interface text uses shared typography and inherits its parent's visual style.
pub(crate) trait UiTextExt: ParentElement + Sized {
    fn child_text(self, text: impl Into<SharedString>) -> Self {
        self.child(role_text(text, FontRole::Ui))
    }
}

impl<T: ParentElement> UiTextExt for T {}
