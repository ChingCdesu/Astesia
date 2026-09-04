use gpui::{Entity, FocusHandle, Focusable as _, Subscription};
use zed_ui::prelude::*;

use crate::application::{DropObjectTarget, QueryTarget};
use crate::db::{FunctionInfo, ProcedureInfo, ViewInfo};

use super::localization::text;
use super::shell::ShellSettings;
use super::sql_language;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ObjectDefinitionKind {
    View,
    Function,
    Procedure,
}

#[derive(Clone, Debug)]
pub(super) struct ObjectDefinition {
    pub(super) target: QueryTarget,
    pub(super) kind: ObjectDefinitionKind,
    pub(super) name: String,
    pub(super) language: Option<String>,
    pub(super) return_type: Option<String>,
    pub(super) definition: Option<String>,
}

impl ObjectDefinition {
    pub(super) fn view(target: QueryTarget, view: &ViewInfo) -> Self {
        Self {
            target,
            kind: ObjectDefinitionKind::View,
            name: view.name.clone(),
            language: None,
            return_type: None,
            definition: view.definition.clone(),
        }
    }

    pub(super) fn function(target: QueryTarget, function: &FunctionInfo) -> Self {
        Self {
            target,
            kind: ObjectDefinitionKind::Function,
            name: function.name.clone(),
            language: function.language.clone(),
            return_type: function.return_type.clone(),
            definition: function.definition.clone(),
        }
    }

    pub(super) fn procedure(target: QueryTarget, procedure: &ProcedureInfo) -> Self {
        Self {
            target,
            kind: ObjectDefinitionKind::Procedure,
            name: procedure.name.clone(),
            language: procedure.language.clone(),
            return_type: None,
            definition: procedure.definition.clone(),
        }
    }

    pub(super) fn drop_target(&self) -> DropObjectTarget {
        match self.kind {
            ObjectDefinitionKind::View => DropObjectTarget::View(self.name.clone()),
            ObjectDefinitionKind::Function => DropObjectTarget::Function(self.name.clone()),
            ObjectDefinitionKind::Procedure => DropObjectTarget::Procedure(self.name.clone()),
        }
    }
}

pub(super) struct ObjectDefinitionItem {
    object: ObjectDefinition,
    editor: Option<Entity<editor::Editor>>,
    invalidation_reason: Option<String>,
    focus_handle: FocusHandle,
    settings: Entity<ShellSettings>,
    _settings_observation: Subscription,
}

impl ObjectDefinitionItem {
    pub(super) fn new(
        object: ObjectDefinition,
        settings: Entity<ShellSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = object.definition.as_ref().map(|definition| {
            let editor = cx.new(|cx| sql_language::editor(definition, window, cx));
            editor.update(cx, |editor, cx| {
                editor.set_read_only(true);
                cx.notify();
            });
            editor
        });
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        Self {
            object,
            editor,
            invalidation_reason: None,
            focus_handle: cx.focus_handle(),
            settings,
            _settings_observation: settings_observation,
        }
    }

    pub(super) fn label(&self) -> String {
        format!(
            "{} · {}/{}",
            self.object.name, self.object.target.connection_name, self.object.target.database
        )
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = &self.editor {
            window.focus(&editor.read(cx).focus_handle(cx), cx);
        } else {
            window.focus(&self.focus_handle, cx);
        }
    }

    pub(super) fn invalidate_session(
        &mut self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<Self>,
    ) {
        if self.object.target.connection_id == connection_id
            && self.object.target.session_generation == session_generation
            && self.invalidation_reason.is_none()
        {
            let language = self.settings.read(cx).language();
            self.invalidation_reason = Some(
                text(
                    language,
                    "连接会话已更改；此定义是先前会话的只读快照。",
                    "The connection session changed; this definition is a read-only snapshot from the previous session.",
                )
                .to_string(),
            );
            cx.notify();
        }
    }

    fn kind_label(&self, language: crate::platform::UiLanguage) -> &'static str {
        match self.object.kind {
            ObjectDefinitionKind::View => text(language, "视图", "View"),
            ObjectDefinitionKind::Function => text(language, "函数", "Function"),
            ObjectDefinitionKind::Procedure => text(language, "存储过程", "Procedure"),
        }
    }
}

impl Render for ObjectDefinitionItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let language = self.settings.read(cx).language();
        let target_label = format!(
            "{} / {}",
            self.object.target.connection_name, self.object.target.database
        );
        let content = match &self.editor {
            Some(editor) => div()
                .flex_1()
                .min_h_0()
                .p_2()
                .child(editor.clone())
                .into_any_element(),
            None => v_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .gap_1()
                .p_6()
                .child(
                    Label::new(text(
                        language,
                        "该对象没有可用的定义",
                        "No definition is available for this object",
                    ))
                    .size(LabelSize::Small),
                )
                .child(
                    Label::new(self.object.name.clone())
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .into_any_element(),
        };

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("ObjectDefinitionItem")
            .size_full()
            .overflow_hidden()
            .child(
                h_flex()
                    .h(px(40.0))
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(
                        Label::new(self.kind_label(language))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(self.object.name.clone())
                            .size(LabelSize::Small)
                            .weight(gpui::FontWeight::MEDIUM)
                            .truncate(),
                    )
                    .child(
                        Label::new(target_label)
                            .flex_1()
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .truncate(),
                    )
                    .when_some(self.object.language.clone(), |element, value| {
                        element.child(
                            Label::new(value)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                    })
                    .when_some(self.object.return_type.clone(), |element, value| {
                        element.child(
                            Label::new(value)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                    }),
            )
            .when_some(self.invalidation_reason.clone(), |element, reason| {
                element.child(
                    h_flex()
                        .px_3()
                        .py_1()
                        .border_b_1()
                        .border_color(colors.border)
                        .bg(status.warning_background)
                        .child(
                            Label::new(reason)
                                .size(LabelSize::XSmall)
                                .color(Color::Warning),
                        ),
                )
            })
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbType;

    fn target() -> QueryTarget {
        QueryTarget {
            connection_id: "primary".to_string(),
            connection_name: "Primary".to_string(),
            database: "app".to_string(),
            db_type: DbType::PostgreSQL,
            session_generation: 4,
        }
    }

    #[test]
    fn definitions_preserve_qualified_identity_and_missing_content() {
        let view = ObjectDefinition::view(
            target(),
            &ViewInfo {
                name: "reporting.monthly_sales".to_string(),
                definition: None,
            },
        );
        assert_eq!(view.target.database, "app");
        assert_eq!(view.name, "reporting.monthly_sales");
        assert_eq!(view.kind, ObjectDefinitionKind::View);
        assert_eq!(view.definition, None);

        let function = ObjectDefinition::function(
            target(),
            &FunctionInfo {
                name: "billing.total".to_string(),
                language: Some("plpgsql".to_string()),
                return_type: Some("numeric".to_string()),
                definition: Some("CREATE FUNCTION billing.total()".to_string()),
            },
        );
        assert_eq!(function.name, "billing.total");
        assert_eq!(function.language.as_deref(), Some("plpgsql"));
        assert_eq!(function.return_type.as_deref(), Some("numeric"));
    }
}
