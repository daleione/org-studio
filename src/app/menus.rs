use crate::{
    editor::{Copy, Cut, Paste, Redo, SelectAll, Undo},
    preview::{
        DecreaseContentFontSize, ExportDocument, IncreaseContentFontSize, OpenDocument,
        QuitApplication, ReloadDocument, ResetContentFontSize, SaveDocument, SaveDocumentAs,
        ShowEditor, ShowHome, ShowReading, ShowSplit, ToggleMinimap, ToggleSidebar, ToggleSoftWrap,
        UseChinese, UseEnglish,
    },
};
use gpui::{Menu, MenuItem, SystemMenuType};

pub fn application_menus(language: crate::i18n::Language) -> Vec<Menu> {
    let t = |zh, en| match language {
        crate::i18n::Language::Chinese => zh,
        crate::i18n::Language::English => en,
    };
    vec![
        Menu::new("Org Studio").items([
            MenuItem::os_submenu(t("服务", "Services"), SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action(t("退出 Org Studio", "Quit Org Studio"), QuitApplication),
        ]),
        Menu::new(t("文件", "File")).items([
            MenuItem::action(t("新建文档", "New document"), crate::app::NewDocument),
            MenuItem::action(t("切换文档", "Switch document"), crate::app::SwitchBuffer),
            MenuItem::action(t("首页", "Home"), ShowHome),
            MenuItem::separator(),
            MenuItem::action(t("打开…", "Open..."), OpenDocument),
            MenuItem::separator(),
            MenuItem::action(t("保存", "Save"), SaveDocument),
            MenuItem::action(t("另存为…", "Save As..."), SaveDocumentAs),
            MenuItem::action(t("保存审阅…", "Review saves..."), crate::app::SaveBuffers),
            MenuItem::action(t("关闭文档", "Close document"), crate::app::CloseBuffer),
            MenuItem::separator(),
            MenuItem::action(t("导出…", "Export..."), ExportDocument),
            MenuItem::separator(),
            MenuItem::action(t("重新载入", "Reload"), ReloadDocument),
        ]),
        Menu::new(t("编辑", "Edit")).items([
            MenuItem::action(t("撤销", "Undo"), Undo),
            MenuItem::action(t("重做", "Redo"), Redo),
            MenuItem::separator(),
            MenuItem::action(t("剪切", "Cut"), Cut),
            MenuItem::action(t("复制", "Copy"), Copy),
            MenuItem::action(t("粘贴", "Paste"), Paste),
            MenuItem::separator(),
            MenuItem::action(t("全选", "Select All"), SelectAll),
            MenuItem::separator(),
            MenuItem::action(t("查找…", "Find…"), crate::app::FindDocument),
        ]),
        Menu::new(t("视图", "View")).items([
            MenuItem::action(t("编辑", "Editor"), ShowEditor),
            MenuItem::action(t("阅读", "Reading"), ShowReading),
            MenuItem::action(t("分屏", "Split"), ShowSplit),
            MenuItem::separator(),
            MenuItem::action(t("自动换行", "Toggle Soft Wrap"), ToggleSoftWrap),
            MenuItem::action(t("侧边栏", "Toggle Sidebar"), ToggleSidebar),
            MenuItem::action(t("缩略图", "Toggle Minimap"), ToggleMinimap),
            MenuItem::separator(),
            MenuItem::action(
                t("增大字号", "Increase Content Font Size"),
                IncreaseContentFontSize,
            ),
            MenuItem::action(
                t("减小字号", "Decrease Content Font Size"),
                DecreaseContentFontSize,
            ),
            MenuItem::action(
                t("重置字号", "Reset Content Font Size"),
                ResetContentFontSize,
            ),
        ]),
        Menu::new(t("语言", "Language")).items([
            MenuItem::action(t("English", "English"), UseEnglish),
            MenuItem::action(t("简体中文", "Chinese (Simplified)"), UseChinese),
        ]),
    ]
}
