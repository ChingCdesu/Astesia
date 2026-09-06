use super::*;
use crate::ui::components::{ContextMenu, ContextMenuEntry, ContextMenuItem};
use gpui_kit::Focusable;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct ProfileMenuState {
    profile_id: String,
    connected: bool,
    operation: Option<ProfileOperationKind>,
    actions: ProfileActions,
    language: crate::platform::UiLanguage,
}

impl ConnectionProfilesPanel {
    fn current_profile_menu_state(&self, cx: &App) -> Option<ProfileMenuState> {
        let profile = self.selected_profile()?;
        Some(ProfileMenuState {
            profile_id: profile.profile.id.clone(),
            connected: profile.session.is_connected(),
            operation: self.state.operation(&profile.profile.id),
            actions: self.profile_actions(),
            language: self.settings.read(cx).language(),
        })
    }

    pub(super) fn refresh_profile_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(previous) = self.profile_menu_state.clone() else {
            return;
        };
        let current = self.current_profile_menu_state(cx);
        if current.as_ref() == Some(&previous) {
            return;
        }
        let Some((_, position, _)) = self.context_menu.as_ref() else {
            self.profile_menu_state = None;
            return;
        };
        let position = *position;
        if current
            .as_ref()
            .is_some_and(|state| state.profile_id == previous.profile_id)
        {
            self.open_profile_menu(previous.profile_id, position, window, cx);
        } else {
            self.context_menu = None;
            self.profile_menu_state = None;
        }
    }

    pub(super) fn open_profile_menu(
        &mut self,
        profile_id: String,
        position: gpui_kit::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_profile_id(profile_id.clone(), cx);
        self.profile_menu_state = self.current_profile_menu_state(cx);
        let Some(profile) = self.selected_profile() else {
            return;
        };
        let language = self.settings.read(cx).language();
        let connected = profile.session.is_connected();
        let operation = self.state.operation(&profile_id);
        let session_action = match operation {
            Some(ProfileOperationKind::Connecting) => {
                Some(text(language, "连接中…", "Connecting…"))
            }
            Some(ProfileOperationKind::Disconnecting) => {
                Some(text(language, "断开中…", "Disconnecting…"))
            }
            _ if connected => Some(text(language, "断开连接", "Disconnect")),
            _ => None,
        };
        let actions = self.profile_actions();
        let owner = cx.entity().downgrade();
        let disconnect_owner = owner.clone();
        let edit_owner = owner.clone();
        let disconnect_id = profile_id.clone();
        let edit_id = profile_id.clone();
        let menu = ContextMenu::build(window, cx, move |menu, _, _| {
            menu.when_some(session_action, |menu, label| {
                menu.item(profile_menu_entry(
                    label,
                    actions.disconnect,
                    move |window, cx| {
                        disconnect_owner
                            .update(cx, |panel, cx| {
                                panel.select_profile_id(disconnect_id.clone(), cx);
                                panel.disconnect_selected(window, cx);
                            })
                            .ok();
                    },
                ))
            })
            .item(profile_menu_entry(
                text(language, "编辑连接…", "Edit Connection…"),
                actions.edit,
                move |window, cx| {
                    edit_owner
                        .update(cx, |panel, cx| {
                            panel.select_profile_id(edit_id.clone(), cx);
                            panel.edit_selected(window, cx);
                        })
                        .ok();
                },
            ))
            .separator()
            .when_else(
                !actions.delete,
                |menu| {
                    menu.item(profile_menu_entry(
                        text(language, "删除连接…", "Delete Connection…"),
                        false,
                        |_, _| {},
                    ))
                },
                |menu| {
                    menu.custom_entry(
                        move |_, _| {
                            div()
                                .id("delete-profile-label")
                                .role(gpui_kit::Role::Label)
                                .aria_value(text(language, "删除连接…", "Delete Connection…"))
                                .child(
                                    Label::new(text(language, "删除连接…", "Delete Connection…"))
                                        .color(Color::Error),
                                )
                                .into_any_element()
                        },
                        move |window, cx| {
                            owner
                                .update(cx, |panel, cx| {
                                    panel.select_profile_id(profile_id.clone(), cx);
                                    panel.confirm_delete_selected(window, cx);
                                })
                                .ok();
                        },
                    )
                },
            )
        });
        let previous_focus = Some(self.selected_profile_focus.clone());
        window.focus(&menu.focus_handle(cx), cx);
        let subscription = cx.subscribe_in(
            &menu,
            window,
            move |panel, menu, _: &gpui_kit::DismissEvent, window, cx| {
                if menu.focus_handle(cx).contains_focused(window, cx) {
                    if let Some(focus) = previous_focus.as_ref() {
                        window.focus(focus, cx);
                    }
                }
                panel.context_menu = None;
                panel.profile_menu_state = None;
                cx.notify();
            },
        );
        self.context_menu = Some((menu, position, subscription));
        cx.notify();
    }
}

fn profile_menu_entry(
    label: &'static str,
    enabled: bool,
    handler: impl Fn(&mut Window, &mut App) + 'static,
) -> ContextMenuItem {
    ContextMenuEntry::new(label)
        .disabled(!enabled)
        .handler(handler)
}
