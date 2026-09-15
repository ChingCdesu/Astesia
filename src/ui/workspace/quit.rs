use super::*;

impl AstesiaRoot {
    pub(in crate::ui) fn install_quit_handler(root: gpui_kit::WeakEntity<Self>, cx: &mut App) {
        cx.set_menus([gpui_kit::Menu::new("Astesia")
            .items([gpui_kit::MenuItem::action("Quit Astesia", QuitApplication)])]);
        // Register at app scope so modal and text-input focus cannot hide the quit action.
        cx.on_action(move |_: &QuitApplication, cx| {
            let root = root.clone();
            cx.defer(move |cx| {
                let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied())
                else {
                    cx.quit();
                    return;
                };
                let _ = window.update(cx, |_, window, cx| {
                    let _ = root.update(cx, |root, cx| match &root.phase {
                        AppPhase::Ready(workspace) => workspace.update(cx, |workspace, cx| {
                            workspace.quit_application(window, cx);
                        }),
                        AppPhase::Loading | AppPhase::Failed(_) => cx.quit(),
                    });
                });
            });
        });
    }
}

impl AstesiaWorkspace {
    fn quit_application(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_prompt() {
            return;
        }
        let unsaved = self
            .workspace_tabs
            .iter()
            .filter(|tab| tab.item.has_unsaved_changes(cx))
            .count();
        if unsaved == 0 {
            cx.quit();
            return;
        }
        let language = self.settings.read(cx).language();
        let detail = format!(
            "{} {unsaved}",
            text(
                language,
                "退出将放弃未保存的标签页数量：",
                "Quitting will discard this many unsaved tabs:"
            )
        );
        let answer = window.prompt(
            PromptLevel::Warning,
            text(language, "退出 Astesia？", "Quit Astesia?"),
            Some(&detail),
            &[
                PromptButton::ok(text(language, "放弃并退出", "Discard and Quit")),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |workspace, cx| {
            if answer.await.ok() == Some(0) {
                let _ = workspace.update(cx, |_, cx| cx.quit());
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    #[gpui_kit::test]
    fn cancelling_quit_preserves_unsaved_tabs_and_allows_another_request(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use crate::{
            connection_repository::SharedConnectionRepository,
            credential_vault::test_support::MemoryCredentialVault,
        };
        cx.update(|cx| crate::ui::initialize_editor_runtime(ThemePreference::Dark, cx));
        let directory = tempfile::tempdir().unwrap();
        let repository = SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        );
        let application = Arc::new(Application::with_repository(repository));
        let settings = cx.new(|_| ShellSettings::new(DesktopPreferences::default(), None));
        let notifications = cx.new(|_| NotificationCenter::new());
        let window = cx.add_window(|window, cx| {
            let profiles = cx.new(|cx| {
                ConnectionProfilesPanel::new(application.clone(), settings.clone(), window, cx)
            });
            AstesiaWorkspace::new(application, profiles, settings, notifications, window, cx)
        });
        window
            .update(cx, |workspace, window, cx| {
                workspace.new_query_tab(window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "x");
        cx.run_until_parked();
        for _ in 0..2 {
            window
                .update(cx, |workspace, window, cx| {
                    workspace.quit_application(window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            assert!(cx.has_pending_prompt());
            cx.simulate_prompt_answer("取消");
            cx.run_until_parked();
            assert!(!cx.has_pending_prompt());
            window
                .update(cx, |workspace, _, cx| {
                    assert_eq!(workspace.workspace_tabs.len(), 1);
                    assert!(workspace.workspace_tabs[0].item.has_unsaved_changes(cx));
                })
                .unwrap();
        }
    }

    #[gpui_kit::test]
    fn cmd_q_reaches_the_app_action_while_editing_text(cx: &mut gpui_kit::TestAppContext) {
        let received = Rc::new(Cell::new(false));
        let observed = received.clone();
        cx.update(|cx| {
            crate::ui::initialize_editor_runtime(ThemePreference::Dark, cx);
            cx.on_action(move |_: &QuitApplication, _| observed.set(true));
        });
        let window = cx.add_window(|window, cx| {
            crate::ui::text_editor::Editor::code("SELECT 1", "sql", window, cx)
        });
        window
            .update(cx, |editor, window, cx| {
                window.focus(&editor.focus_handle(cx), cx)
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "cmd-q");
        assert!(received.get());
    }
}
