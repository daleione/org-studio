use crate::{command::CommandRegistry, i18n::Language, input::WhichKeyCandidate};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Group {
    File,
    Document,
    Navigation,
    View,
    Org,
    Commands,
}

pub(super) fn text(language: Language, zh: &'static str, en: &'static str) -> &'static str {
    match language {
        Language::Chinese => zh,
        Language::English => en,
    }
}

impl Group {
    pub fn title(self, language: Language) -> &'static str {
        match self {
            Self::File => text(language, "文件", "Files"),
            Self::Document => text(language, "文档", "Documents"),
            Self::Navigation => text(language, "导航", "Navigation"),
            Self::View => text(language, "视图", "View"),
            Self::Org => "Org",
            Self::Commands => text(language, "命令", "Commands"),
        }
    }
}

pub(super) struct Item {
    pub key: String,
    pub title: String,
    pub group: Group,
    pub prefix: bool,
    pub disabled: bool,
}

pub(super) fn prefix_group(prefix: &str) -> Option<Group> {
    match prefix {
        "C-c v" => Some(Group::View),
        "C-c C-x" => Some(Group::Org),
        _ => None,
    }
}

pub(super) fn item(
    prefix: &str,
    candidate: &WhichKeyCandidate,
    commands: &CommandRegistry,
    language: Language,
) -> Item {
    let descriptor = candidate.command.and_then(|key| commands.descriptor(key));
    let name = descriptor.map_or("", |d| d.name.as_str());
    let (group, zh, en) = match name {
        "org-studio.workspace.open-file" => (Group::File, "打开文件", "Open file"),
        "org-studio.document.save" => (Group::File, "保存文件", "Save file"),
        "org-studio.document.save-as" => (Group::File, "另存为", "Save as"),
        "org-studio.document.reload" => (Group::File, "重新加载", "Reload file"),
        "org-studio.document.export" => (Group::File, "导出文档", "Export document"),
        "org-studio.workspace.switch-buffer" => (Group::Document, "切换文档", "Switch document"),
        "org-studio.workspace.close-buffer" => (Group::Document, "关闭文档", "Close document"),
        "org-studio.workspace.save-buffers" => (Group::Document, "保存多个文档", "Review saves"),
        "org-studio.workspace.next-buffer" => (Group::Navigation, "下一个文档", "Next document"),
        "org-studio.workspace.previous-buffer" => {
            (Group::Navigation, "上一个文档", "Previous document")
        }
        "org-studio.dired.open-default" | "org-studio.file-manager.open" => {
            (Group::Navigation, "文件管理", "File manager")
        }
        "org-studio.file-manager.toggle-sidebar" => {
            (Group::Navigation, "切换侧边栏", "Toggle sidebar")
        }
        "org-studio.workspace.show-home" => (Group::Navigation, "首页", "Home"),
        "org-studio.application.quit" => (Group::Navigation, "退出应用", "Quit application"),
        "org-studio.workspace.show-editor" => (Group::View, "编辑", "Edit"),
        "org-studio.workspace.show-reading" => (Group::View, "阅读", "Read"),
        "org-studio.workspace.show-split" => (Group::View, "分屏", "Split"),
        "org-studio.workspace.open-agenda" => (Group::Org, "日程", "Agenda"),
        "org-studio.workspace.open-agenda-text" => (Group::Org, "日程文本", "Agenda text"),
        "org-studio.file-manager.return-document" => {
            (Group::Navigation, "返回文档", "Return to document")
        }
        "org-studio.org.context-command" => (Group::Org, "执行上下文操作", "Context action"),
        "org-studio.org.toggle-inline-image-previews" => {
            (Group::Org, "切换行内图片", "Toggle inline images")
        }
        "org-studio.babel.execute-source-block" => (Group::Org, "执行代码块", "Run source block"),
        _ => (Group::Commands, "", ""),
    };
    let (group, title) = if candidate.is_prefix {
        let group = prefix_group(&format!("{prefix} {}", candidate.key)).unwrap_or(Group::Commands);
        (group, group.title(language).to_owned())
    } else if !zh.is_empty() {
        (group, text(language, zh, en).to_owned())
    } else {
        (
            group,
            descriptor
                .map(|d| d.title.to_string())
                .unwrap_or_else(|| text(language, "不可用", "Unavailable").to_owned()),
        )
    };
    Item {
        key: candidate.key.to_string(),
        title,
        group,
        prefix: candidate.is_prefix,
        disabled: candidate.disabled,
    }
}
