use super::{BuiltinCommand, OrgDisplayFeature};

pub(crate) struct OrgEditorCommand {
    pub(crate) name: &'static str,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) label: &'static str,
    pub(crate) command: BuiltinCommand,
}

impl OrgEditorCommand {
    pub(crate) fn edits_source(&self) -> bool {
        self.command == BuiltinCommand::EditLinkAt
    }
}

pub(crate) const ORG_EDITOR_COMMANDS: &[OrgEditorCommand] = &[
    OrgEditorCommand {
        name: "org-studio.org.edit-link",
        aliases: &["org-insert-link"],
        label: "link.edit",
        command: BuiltinCommand::EditLinkAt,
    },
    OrgEditorCommand {
        name: "org-studio.org.toggle-link-display",
        aliases: &["org-toggle-link-display"],
        label: "display.links",
        command: BuiltinCommand::ToggleOrgDisplay(OrgDisplayFeature::DescriptiveLinks),
    },
    OrgEditorCommand {
        name: "org-studio.org.toggle-leading-stars",
        aliases: &["org-toggle-leading-stars"],
        label: "display.stars",
        command: BuiltinCommand::ToggleOrgDisplay(OrgDisplayFeature::LeadingStars),
    },
    OrgEditorCommand {
        name: "org-studio.org.toggle-heading-bullets",
        aliases: &["org-toggle-heading-bullets"],
        label: "display.heading_bullets",
        command: BuiltinCommand::ToggleOrgDisplay(OrgDisplayFeature::HeadingBullets),
    },
    OrgEditorCommand {
        name: "org-studio.org.toggle-emphasis-markers",
        aliases: &["org-toggle-emphasis-markers"],
        label: "display.emphasis",
        command: BuiltinCommand::ToggleOrgDisplay(OrgDisplayFeature::EmphasisMarkers),
    },
    OrgEditorCommand {
        name: "org-studio.org.toggle-pretty-entities",
        aliases: &["org-toggle-pretty-entities"],
        label: "display.entities",
        command: BuiltinCommand::ToggleOrgDisplay(OrgDisplayFeature::Entities),
    },
];
