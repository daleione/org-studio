//! Org display directives are resolved without changing other language caches.

use super::{EditorStyleId, EditorSyntaxService, Language, classify_line};
use crate::document::{DocumentSnapshot, TextSnapshot};
use std::path::Path;

impl EditorSyntaxService {
    pub(in crate::editor) fn display_options(
        &self,
        path: &Path,
        snapshot: &DocumentSnapshot,
    ) -> crate::org_semantic::display::OrgDisplayOptions {
        use crate::org_semantic::display::{OrgDisplayConfig, OrgDisplayOptions};
        if crate::document::DocumentFormat::detect(path)
            != Some(crate::document::DocumentFormat::Org)
        {
            return OrgDisplayOptions::default();
        }
        let directives = {
            let mut cache = self
                .display_config
                .lock()
                .expect("display config cache poisoned");
            if let Some(config) = cache.as_ref()
                && config.document_id == snapshot.document_id()
                && config.revision > snapshot.revision()
            {
                OrgDisplayConfig::new(snapshot).directives
            } else {
                if cache.as_ref().is_none_or(|config| {
                    config.document_id != snapshot.document_id()
                        || config.revision != snapshot.revision()
                }) {
                    *cache = Some(OrgDisplayConfig::new(snapshot));
                }
                cache.as_ref().unwrap().directives.clone()
            }
        };
        let mut options = OrgDisplayOptions::default();
        for directive in directives {
            let line = snapshot.line_of_byte(directive.source.range.start);
            let mut context = self.context_at(snapshot, Language::Org, line);
            let source = snapshot.copy_range(directive.source.range);
            if classify_line(Language::Org, &source, &mut context) == EditorStyleId::Meta {
                options.apply_startup(&directive.value);
            }
        }
        options
    }

    pub(in crate::editor) fn update_display_config(
        &self,
        snapshot: &DocumentSnapshot,
        delta: &crate::document::RevisionDelta,
    ) {
        if let Some(config) = self
            .display_config
            .lock()
            .expect("display config cache poisoned")
            .as_mut()
        {
            config.apply_delta(snapshot, delta);
        }
    }
}
