use gpui::{FocusHandle, Focusable, Render};
use workspace::{DismissDecision, ModalView};
use zed_ui::{prelude::*, ElevationIndex, Modal, ModalFooter, ModalHeader, Section, TintColor};

use super::{
    ConnectionProfileForm, FormNotice, FormOperation, NoticeKind, SubmitConnectionProfile,
};
use crate::db::DbType;
use crate::ui::engine_presentation::engine_label;
use crate::ui::localization::text;

impl ConnectionProfileForm {
    fn render_engine_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(
                Label::new(text(self.language, "数据库类型", "Database Type"))
                    .size(LabelSize::Small),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .children(
                        DbType::all()
                            .into_iter()
                            .enumerate()
                            .map(|(index, db_type)| {
                                let label = engine_label(db_type);
                                let selected = self.db_type == db_type;
                                div()
                                    .key_context("ConnectionProfileFormControl")
                                    .on_action(cx.listener(
                                        move |form, _: &menu::Confirm, window, cx| {
                                            form.select_db_type(db_type, window, cx);
                                        },
                                    ))
                                    .child(
                                        Button::new(format!("connection-engine-{label}"), label)
                                            .size(ButtonSize::Compact)
                                            .tab_index(2 + index as isize)
                                            .toggle_state(selected)
                                            .selected_style(ButtonStyle::Tinted(TintColor::Accent))
                                            .disabled(self.operation.is_busy())
                                            .on_click(cx.listener(move |form, _, window, cx| {
                                                form.select_db_type(db_type, window, cx);
                                            })),
                                    )
                            }),
                    ),
            )
    }

    fn render_notice(&self, notice: &FormNotice, cx: &mut Context<Self>) -> AnyElement {
        let (color, icon) = match notice.kind {
            NoticeKind::Success => (Color::Success, IconName::Check),
            NoticeKind::Warning => (Color::Warning, IconName::Warning),
            NoticeKind::Error => (Color::Error, IconName::Warning),
        };
        let status = cx.theme().status();
        let (border, background) = match notice.kind {
            NoticeKind::Success => (status.success_border, status.success_background),
            NoticeKind::Warning => (status.warning_border, status.warning_background),
            NoticeKind::Error => (status.error_border, status.error_background),
        };

        v_flex()
            .gap_1()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(background)
            .child(
                h_flex()
                    .gap_2()
                    .child(Icon::new(icon).size(IconSize::Small).color(color))
                    .child(Label::new(notice.message.clone()).size(LabelSize::Small)),
            )
            .when_some(notice.detail.clone(), |element, detail| {
                element.child(Label::new(detail).size(LabelSize::XSmall).line_clamp(3))
            })
            .into_any_element()
    }
}

impl Render for ConnectionProfileForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editing = self.origin.is_editing();
        let removes_credential = self.origin.removes_saved_credential(self.db_type);
        let title = if editing {
            text(self.language, "编辑连接", "Edit Connection")
        } else {
            text(self.language, "新建连接", "New Connection")
        };
        let description = if removes_credential {
            text(
                self.language,
                "保存为 SQLite 时会移除此前保存的密码。",
                "Saving as SQLite removes the previously stored password.",
            )
        } else if editing {
            text(
                self.language,
                "修改连接配置；密码留空时保留已保存的凭据。",
                "Update the profile; leave the password blank to keep the stored credential.",
            )
        } else {
            text(
                self.language,
                "配置数据库端点与可选的组织信息。",
                "Configure the database endpoint and optional organization details.",
            )
        };
        let busy = self.operation.is_busy();
        let spec = self.db_type.profile_spec();
        let test_notice = self
            .test_notice
            .as_ref()
            .map(|notice| self.render_notice(notice, cx));
        let save_notice = self
            .save_notice
            .as_ref()
            .map(|notice| self.render_notice(notice, cx));
        let credential_notice = removes_credential.then(|| {
            self.render_notice(
                &FormNotice::warning(text(
                    self.language,
                    "保存为 SQLite 时将移除此前保存的密码。",
                    "Saving as SQLite will remove the stored password.",
                )),
                cx,
            )
        });

        div()
            .key_context("ConnectionProfileForm")
            .tab_group()
            .track_focus(&self.focus_handle(cx))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::submit_action))
            .on_action(cx.listener(Self::focus_next))
            .on_action(cx.listener(Self::focus_previous))
            .elevation_3(cx)
            .occlude()
            .w(rems(42.0))
            .max_h(rems(44.0))
            .child(
                Modal::new("connection-profile-form", Some(self.scroll_handle.clone()))
                    .header(
                        ModalHeader::new()
                            .headline(title)
                            .description(description)
                            .show_dismiss_button(!busy),
                    )
                    .section(
                        Section::new()
                            .child(
                                v_flex()
                                    .gap_3()
                                    .child(self.fields.name.clone())
                                    .child(self.render_engine_picker(cx)),
                            )
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(self.fields.endpoint.clone()),
                                    )
                                    .when(!spec.is_file(), |element| {
                                        element.child(
                                            div().w(rems(9.0)).child(self.fields.port.clone()),
                                        )
                                    }),
                            )
                            .when(!spec.is_file(), |section| {
                                section.child(
                                    h_flex()
                                        .items_start()
                                        .gap_3()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .child(self.fields.username.clone()),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .child(self.fields.password.clone()),
                                        ),
                                )
                            })
                            .when(!spec.is_file(), |section| {
                                section.child(self.fields.database.clone())
                            })
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(self.fields.group_name.clone()),
                                    )
                                    .child(
                                        div()
                                            .w(rems(12.0))
                                            .min_w_0()
                                            .child(self.fields.color.clone()),
                                    ),
                            )
                            .child(self.fields.tags.clone())
                            .children(credential_notice)
                            .children(test_notice)
                            .children(save_notice),
                    )
                    .footer(
                        ModalFooter::new().end_slot(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .key_context("ConnectionProfileFormControl")
                                        .on_action(cx.listener(Self::cancel_confirm))
                                        .child(
                                            Button::new(
                                                "cancel-connection-profile",
                                                text(self.language, "取消", "Cancel"),
                                            )
                                            .tab_index(17_isize)
                                            .disabled(busy)
                                            .on_click(cx.listener(Self::cancel_click)),
                                        ),
                                )
                                .child(
                                    div()
                                        .key_context("ConnectionProfileFormControl")
                                        .on_action(cx.listener(Self::test_confirm))
                                        .child(
                                            Button::new(
                                                "test-connection-profile",
                                                text(self.language, "测试连接", "Test Connection"),
                                            )
                                            .tab_index(18_isize)
                                            .style(ButtonStyle::Outlined)
                                            .loading(self.operation == FormOperation::Testing)
                                            .disabled(busy)
                                            .on_click(cx.listener(Self::test_click)),
                                        ),
                                )
                                .child(
                                    div()
                                        .key_context("ConnectionProfileFormControl")
                                        .on_action(cx.listener(Self::save_confirm))
                                        .child(
                                            Button::new(
                                                "save-connection-profile",
                                                text(self.language, "保存", "Save"),
                                            )
                                            .tab_index(19_isize)
                                            .style(ButtonStyle::Filled)
                                            .layer(ElevationIndex::ModalSurface)
                                            .loading(self.operation == FormOperation::Saving)
                                            .disabled(busy)
                                            .key_binding(zed_ui::KeyBinding::for_action(
                                                &SubmitConnectionProfile,
                                                cx,
                                            ))
                                            .on_click(cx.listener(Self::save_click)),
                                        ),
                                ),
                        ),
                    ),
            )
    }
}

impl Focusable for ConnectionProfileForm {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.fields.name.read(cx).focus_handle(cx)
    }
}

impl ModalView for ConnectionProfileForm {
    fn fade_out_background(&self) -> bool {
        true
    }

    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut Context<Self>) -> DismissDecision {
        DismissDecision::Dismiss(!self.operation.is_busy())
    }
}
