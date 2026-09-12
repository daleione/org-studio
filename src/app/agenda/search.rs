use crate::app::{
    WorkspaceWindow,
    native_input::{InputConfig, InputEvent, NativeInput},
};
use gpui::{Context, Entity, prelude::*};

impl WorkspaceWindow {
    pub(super) fn create_agenda_search_input(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Entity<NativeInput> {
        let config = InputConfig {
            id: "agenda-search-input",
            placeholder: self.language.text("agenda.search").into(),
            ..Default::default()
        };
        let input = cx.new(|cx| NativeInput::new(config, cx));
        self.agenda.search_input_subscription =
            Some(cx.subscribe(&input, |w, _, event, cx| match event {
                InputEvent::Changed(value) => {
                    w.dispatch_agenda_intent(super::UiIntent::SetSearch(value.clone()), cx)
                }
                InputEvent::Command { .. } => {
                    w.focus_workspace_on_render = true;
                    cx.notify();
                }
                InputEvent::MetricsChanged => {}
            }));
        input
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::WorkspaceWindow;
    use gpui::EntityInputHandler;

    #[gpui::test]
    fn agenda_search_native_typing_ime_and_clear_filter_results(cx: &mut gpui::TestAppContext) {
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        workspace.update(cx, |workspace, _| {
            let snapshot = crate::document::DocumentSnapshot::from_utf8(
                "* TODO 中文任务\n* TODO qckj\n".as_bytes().to_vec(),
            )
            .unwrap();
            let analysis = crate::org_semantic::analyze(
                &snapshot,
                std::sync::Arc::new(crate::org_syntax::parse(&snapshot)),
            );
            workspace
                .agenda
                .runtime
                .index
                .replace(crate::agenda::shard_from_live(
                    crate::agenda::FileId(99),
                    1,
                    std::sync::Arc::new("/tmp/search-test.org".into()),
                    &analysis,
                ));
            workspace.agenda.state.builtin = crate::agenda::BuiltinQuery::Unscheduled;
            workspace.agenda.state.navigation = "Unscheduled".into();
            workspace.agenda.requery();
        });
        let input = workspace.update(cx, |w, cx| w.create_agenda_search_input(cx));
        struct InputView(gpui::Entity<NativeInput>);
        impl gpui::Render for InputView {
            fn render(
                &mut self,
                _: &mut gpui::Window,
                _: &mut gpui::Context<Self>,
            ) -> impl gpui::IntoElement {
                gpui::div().child(self.0.clone())
            }
        }
        let window = cx.add_window(|_, _| InputView(input.clone()));
        let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        cx.update(|window, app| {
            let focus = input.read(app).focus.clone();
            window.focus(&focus, app);
        });
        cx.simulate_keystrokes("q c k j");
        cx.run_until_parked();
        input.update(cx, |input, _| assert_eq!(input.text, "qckj"));
        workspace.update(cx, |workspace, _| {
            assert_eq!(
                workspace
                    .agenda
                    .page_query
                    .result
                    .as_ref()
                    .unwrap()
                    .entries
                    .len(),
                1
            )
        });
        cx.simulate_keystrokes("cmd-a backspace");
        cx.run_until_parked();
        workspace.update(cx, |workspace, _| {
            assert_eq!(
                workspace
                    .agenda
                    .page_query
                    .result
                    .as_ref()
                    .unwrap()
                    .entries
                    .len(),
                2
            )
        });
        cx.update(|window, app| {
            input.update(app, |input, cx| {
                input.replace_and_mark_text_in_range(None, "中", Some(1..1), window, cx);
                input.replace_text_in_range(None, "中文", window, cx);
                assert_eq!(input.selection, 6..6);
                assert!(input.marked.is_none());
            })
        });
        cx.run_until_parked();
        workspace.update(cx, |workspace, _| {
            assert_eq!(&*workspace.agenda.state.search, "中文");
            assert_eq!(
                workspace
                    .agenda
                    .page_query
                    .result
                    .as_ref()
                    .unwrap()
                    .entries
                    .len(),
                1
            );
        });
    }
}
