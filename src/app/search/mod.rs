//! Current-document search: sessions, input, scanning, navigation, replacement and presentation.

gpui::actions!(document_search, [FindDocument]);

mod geometry;
mod input;
mod lifecycle;
mod navigation;
mod notice;
pub(crate) mod presentation;
mod replace;
mod scan;
mod session;
mod view;

#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod tests;

pub(crate) use session::SearchHost;
