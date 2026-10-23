use std::collections::HashSet;

mod confirmation;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(crate) struct MySqlHost {
    pub(crate) confirmation: Option<confirmation::QueryConfirmation>,
    pub(crate) trusted: HashSet<(std::path::PathBuf, String)>,
}
