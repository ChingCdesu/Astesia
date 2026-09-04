use std::sync::Arc;

use editor::Editor;
use gpui::{DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, ScrollHandle, Window};
use ui_input::InputField;
use workspace::{DismissDecision, ModalView};
use zed_ui::{
    prelude::*, Checkbox, ElevationIndex, Modal, ModalFooter, ModalHeader, Section, ToggleState,
};

use crate::application::{
    object_creation_policy, trigger_event_supported, trigger_timing_supported,
    trigger_uses_function_reference, Application, CreateObjectSpec, DatabaseObjectKind,
    ObjectCreationPolicy, ObjectMutation, QueryTarget, TableColumnSpec, TriggerEvent,
    TriggerTiming,
};
use crate::db::TableRef;
use crate::platform::UiLanguage;

use super::localization::text;
use super::sql_language;

mod draft;

use draft::{ColumnDraft, CreateObjectDraft, ObjectMutationFormState, TableFields, TriggerFields};

#[derive(Clone, Debug)]
pub(super) enum ObjectMutationFormMode {
    Create {
        target: QueryTarget,
        kind: DatabaseObjectKind,
        schema: Option<String>,
    },
    Rename {
        target: QueryTarget,
        kind: DatabaseObjectKind,
        original_name: String,
    },
}

impl ObjectMutationFormMode {
    pub(super) fn target(&self) -> &QueryTarget {
        match self {
            Self::Create { target, .. } | Self::Rename { target, .. } => target,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct ObjectMutationSaved {
    pub(super) target: QueryTarget,
    pub(super) kind: DatabaseObjectKind,
    pub(super) identity: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FormOperation {
    Idle,
    Executing,
}

pub(super) struct ObjectMutationForm {
    application: Arc<Application>,
    state: ObjectMutationFormState,
    name: Entity<InputField>,
    operation: FormOperation,
    error: Option<String>,
    scroll_handle: ScrollHandle,
    language_setting: UiLanguage,
}

impl EventEmitter<ObjectMutationSaved> for ObjectMutationForm {}
impl EventEmitter<DismissEvent> for ObjectMutationForm {}

impl ObjectMutationForm {
    pub(super) fn new(
        application: Arc<Application>,
        mode: ObjectMutationFormMode,
        language_setting: UiLanguage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let db_type = mode.target().db_type;
        let name = input(
            window,
            cx,
            text(language_setting, "对象名称", "Object name"),
            text(language_setting, "名称", "Name"),
            1,
        );
        let state = match mode {
            ObjectMutationFormMode::Create {
                target,
                kind,
                schema,
            } => ObjectMutationFormState::Create {
                target,
                schema,
                draft: CreateObjectDraft::new(
                    kind,
                    object_creation_policy(db_type),
                    language_setting,
                    window,
                    cx,
                ),
            },
            ObjectMutationFormMode::Rename {
                target,
                kind,
                original_name,
            } => {
                let initial_name = if kind == DatabaseObjectKind::Table {
                    rename_default_name(&original_name)
                } else {
                    &original_name
                };
                set_text(&name, initial_name, window, cx);
                ObjectMutationFormState::Rename {
                    target,
                    kind,
                    original_name,
                }
            }
        };
        Self {
            application,
            state,
            name,
            operation: FormOperation::Idle,
            error: None,
            scroll_handle: ScrollHandle::new(),
            language_setting,
        }
    }

    fn table_fields_mut(&mut self) -> Option<&mut TableFields> {
        match &mut self.state {
            ObjectMutationFormState::Create {
                draft: CreateObjectDraft::Table(fields),
                ..
            } => Some(fields),
            _ => None,
        }
    }

    fn trigger_fields_mut(&mut self) -> Option<&mut TriggerFields> {
        match &mut self.state {
            ObjectMutationFormState::Create {
                draft: CreateObjectDraft::Trigger(fields),
                ..
            } => Some(fields),
            _ => None,
        }
    }

    fn new_column(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<ColumnDraft> {
        let language = self.language_setting;
        let fields = self.table_fields_mut()?;
        Some(column_draft(
            &mut fields.next_column_id,
            language,
            window,
            cx,
        ))
    }

    fn add_column(&mut self, _: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.operation == FormOperation::Executing {
            return;
        }
        let Some(column) = self.new_column(window, cx) else {
            return;
        };
        let default_column_type =
            object_creation_policy(self.state.target().db_type).default_column_type;
        set_text(&column.data_type, default_column_type, window, cx);
        if let Some(fields) = self.table_fields_mut() {
            fields.columns.push(column);
        }
        cx.notify();
    }

    fn remove_column(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.operation == FormOperation::Idle {
            if let Some(fields) = self.table_fields_mut() {
                if fields.columns.len() > 1 {
                    fields.columns.retain(|column| column.id != id);
                    cx.notify();
                }
            }
        }
    }

    fn set_column_nullable(&mut self, id: u64, state: ToggleState, cx: &mut Context<Self>) {
        let Some(fields) = self.table_fields_mut() else {
            return;
        };
        if let Some(column) = fields.columns.iter_mut().find(|column| column.id == id) {
            column.nullable = state == ToggleState::Selected;
            cx.notify();
        }
    }

    fn set_column_primary_key(&mut self, id: u64, state: ToggleState, cx: &mut Context<Self>) {
        let Some(fields) = self.table_fields_mut() else {
            return;
        };
        if let Some(column) = fields.columns.iter_mut().find(|column| column.id == id) {
            column.primary_key = state == ToggleState::Selected;
            if column.primary_key {
                column.nullable = false;
            }
            cx.notify();
        }
    }

    fn set_trigger_timing(&mut self, timing: TriggerTiming, cx: &mut Context<Self>) {
        if self.operation == FormOperation::Idle {
            if let Some(fields) = self.trigger_fields_mut() {
                fields.timing = timing;
                cx.notify();
            }
        }
    }

    fn set_trigger_event(&mut self, event: TriggerEvent, cx: &mut Context<Self>) {
        if self.operation == FormOperation::Idle {
            if let Some(fields) = self.trigger_fields_mut() {
                fields.event = event;
                cx.notify();
            }
        }
    }

    fn build_mutation(&self, cx: &Context<Self>) -> Result<ObjectMutation, String> {
        let name = field_text(&self.name, cx).trim().to_string();
        if name.is_empty() {
            return Err(
                text(self.language_setting, "名称不能为空", "Name is required").to_string(),
            );
        }
        match &self.state {
            ObjectMutationFormState::Rename {
                kind,
                original_name,
                ..
            } => Ok(ObjectMutation::Rename {
                kind: *kind,
                name: original_name.clone(),
                new_name: name,
            }),
            ObjectMutationFormState::Create {
                target,
                schema,
                draft,
            } => {
                let name = qualify_name(schema.as_deref(), &name);
                let spec = match draft {
                    CreateObjectDraft::Database => CreateObjectSpec::Database { name },
                    CreateObjectDraft::Schema => CreateObjectSpec::Schema { name },
                    CreateObjectDraft::Table(fields) => CreateObjectSpec::Table {
                        name,
                        columns: fields
                            .columns
                            .iter()
                            .map(|column| TableColumnSpec {
                                name: field_text(&column.name, cx).trim().to_string(),
                                data_type: field_text(&column.data_type, cx).trim().to_string(),
                                nullable: column.nullable,
                                primary_key: column.primary_key,
                                default_value: optional_text(&column.default_value, cx),
                            })
                            .collect(),
                    },
                    CreateObjectDraft::View(fields) => CreateObjectSpec::View {
                        name,
                        query: fields.definition.read(cx).text(cx),
                    },
                    CreateObjectDraft::Function(fields) => CreateObjectSpec::Function {
                        name,
                        arguments: field_text(&fields.arguments, cx),
                        return_type: field_text(&fields.return_type, cx),
                        language: field_text(&fields.language, cx),
                        body: fields.definition.read(cx).text(cx),
                    },
                    CreateObjectDraft::Procedure(fields) => CreateObjectSpec::Procedure {
                        name,
                        arguments: field_text(&fields.arguments, cx),
                        language: field_text(&fields.language, cx),
                        body: fields.definition.read(cx).text(cx),
                    },
                    CreateObjectDraft::Trigger(fields) => CreateObjectSpec::Trigger {
                        name,
                        table: TableRef::parse(
                            target.db_type,
                            field_text(&fields.table, cx).trim(),
                        )
                        .map_err(|error| error.to_string())?,
                        timing: fields.timing,
                        event: fields.event,
                        body: fields.definition.read(cx).text(cx),
                    },
                    CreateObjectDraft::User(fields) => CreateObjectSpec::User {
                        name,
                        host: object_creation_policy(target.db_type)
                            .user_host
                            .then(|| field_text(&fields.host, cx)),
                        password: field_text(&fields.password, cx),
                    },
                };
                Ok(ObjectMutation::Create(spec))
            }
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.operation == FormOperation::Executing {
            return;
        }
        let mutation = match self.build_mutation(cx) {
            Ok(mutation) => mutation,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.operation = FormOperation::Executing;
        self.error = None;
        cx.notify();

        let application = self.application.clone();
        let target = self.state.target().clone();
        let kind = self.state.kind();
        let identity = mutation.display_identity();
        let operation = gpui_tokio::Tokio::spawn(cx, {
            let target = target.clone();
            async move { application.objects().execute(&target, &mutation).await }
        });
        cx.spawn_in(window, async move |form, cx| {
            let result = match operation.await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            form.update_in(cx, |form, window, cx| match result {
                Ok(()) => {
                    form.operation = FormOperation::Idle;
                    cx.emit(ObjectMutationSaved {
                        target,
                        kind,
                        identity,
                    });
                    cx.emit(DismissEvent);
                    window.focus(&form.focus_handle(cx), cx);
                }
                Err(error) => {
                    form.operation = FormOperation::Idle;
                    form.error = Some(error);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn cancel(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.operation == FormOperation::Idle {
            cx.emit(DismissEvent);
        }
    }

    fn render_definition_editor(
        &self,
        definition: &Entity<Editor>,
        label: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_1()
            .child(Label::new(label.to_string()).size(LabelSize::Small))
            .child(
                div()
                    .h(rems(12.0))
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded_sm()
                    .p_1()
                    .child(definition.clone()),
            )
            .into_any_element()
    }

    fn render_columns(&self, fields: &TableFields, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.weak_entity();
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        Label::new(text(self.language_setting, "列", "Columns"))
                            .size(LabelSize::Small),
                    )
                    .child(
                        Button::new(
                            "add-object-column",
                            text(self.language_setting, "添加列", "Add Column"),
                        )
                        .size(ButtonSize::Compact)
                        .disabled(self.operation == FormOperation::Executing)
                        .on_click(cx.listener(Self::add_column)),
                    ),
            )
            .children(fields.columns.iter().map(|column| {
                let id = column.id;
                let nullable_weak = weak.clone();
                let primary_weak = weak.clone();
                let remove_weak = weak.clone();
                h_flex()
                    .items_end()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(column.name.clone()))
                    .child(div().flex_1().min_w_0().child(column.data_type.clone()))
                    .child(div().w(rems(10.0)).child(column.default_value.clone()))
                    .child(
                        Checkbox::new(
                            ("column-primary-key", id as usize),
                            toggle_state(column.primary_key),
                        )
                        .label("PK")
                        .disabled(self.operation == FormOperation::Executing)
                        .on_click(move |state, _, cx| {
                            primary_weak
                                .update(cx, |form, cx| form.set_column_primary_key(id, *state, cx))
                                .ok();
                        }),
                    )
                    .child(
                        Checkbox::new(
                            ("column-nullable", id as usize),
                            toggle_state(column.nullable),
                        )
                        .label("NULL")
                        .disabled(self.operation == FormOperation::Executing || column.primary_key)
                        .on_click(move |state, _, cx| {
                            nullable_weak
                                .update(cx, |form, cx| form.set_column_nullable(id, *state, cx))
                                .ok();
                        }),
                    )
                    .child(
                        IconButton::new(("remove-object-column", id as usize), IconName::Trash)
                            .icon_size(IconSize::Small)
                            .disabled(
                                self.operation == FormOperation::Executing
                                    || fields.columns.len() == 1,
                            )
                            .on_click(move |_, _, cx| {
                                remove_weak
                                    .update(cx, |form, cx| form.remove_column(id, cx))
                                    .ok();
                            }),
                    )
            }))
            .into_any_element()
    }

    fn render_create_fields(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut elements = vec![self.name.clone().into_any_element()];
        let ObjectMutationFormState::Create { draft, target, .. } = &self.state else {
            return elements;
        };
        let policy = object_creation_policy(target.db_type);
        match draft {
            CreateObjectDraft::Database | CreateObjectDraft::Schema => {}
            CreateObjectDraft::Table(fields) => elements.push(self.render_columns(fields, cx)),
            CreateObjectDraft::View(fields) => {
                elements.push(self.render_definition_editor(
                    &fields.definition,
                    text(self.language_setting, "SELECT 查询", "SELECT query"),
                    cx,
                ));
            }
            CreateObjectDraft::Function(fields) => {
                elements.push(fields.arguments.clone().into_any_element());
                if policy.function_return_type {
                    elements.push(fields.return_type.clone().into_any_element());
                    if policy.function_language {
                        elements.push(fields.language.clone().into_any_element());
                    }
                }
                elements.push(self.render_definition_editor(
                    &fields.definition,
                    if !policy.function_return_type {
                        text(self.language_setting, "Lambda 表达式", "Lambda expression")
                    } else {
                        text(self.language_setting, "函数体", "Function body")
                    },
                    cx,
                ));
            }
            CreateObjectDraft::Procedure(fields) => {
                elements.push(fields.arguments.clone().into_any_element());
                if policy.procedure_language {
                    elements.push(fields.language.clone().into_any_element());
                }
                elements.push(self.render_definition_editor(
                    &fields.definition,
                    text(self.language_setting, "过程体", "Procedure body"),
                    cx,
                ));
            }
            CreateObjectDraft::Trigger(fields) => {
                elements.push(fields.table.clone().into_any_element());
                elements.push(self.render_trigger_options(fields, cx));
                elements.push(self.render_definition_editor(
                    &fields.definition,
                    if trigger_uses_function_reference(target.db_type) {
                        text(
                            self.language_setting,
                            "触发器函数，例如 audit_row()",
                            "Trigger function, e.g. audit_row()",
                        )
                    } else {
                        text(self.language_setting, "触发器体", "Trigger body")
                    },
                    cx,
                ));
            }
            CreateObjectDraft::User(fields) => {
                if policy.user_host {
                    elements.push(fields.host.clone().into_any_element());
                }
                elements.push(fields.password.clone().into_any_element());
            }
        }
        elements
    }

    fn render_trigger_options(&self, fields: &TriggerFields, cx: &mut Context<Self>) -> AnyElement {
        let db_type = self.state.target().db_type;
        let timings = [
            (TriggerTiming::Before, "BEFORE"),
            (TriggerTiming::After, "AFTER"),
            (TriggerTiming::InsteadOf, "INSTEAD OF"),
        ];
        let events = [
            (TriggerEvent::Insert, "INSERT"),
            (TriggerEvent::Update, "UPDATE"),
            (TriggerEvent::Delete, "DELETE"),
            (TriggerEvent::Truncate, "TRUNCATE"),
        ];
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_1()
                    .children(timings.into_iter().map(|(timing, label)| {
                        Button::new(format!("trigger-timing-{label}"), label)
                            .size(ButtonSize::Compact)
                            .toggle_state(fields.timing == timing)
                            .disabled(
                                self.operation == FormOperation::Executing
                                    || !trigger_timing_supported(db_type, timing),
                            )
                            .on_click(cx.listener(move |form, _, _, cx| {
                                form.set_trigger_timing(timing, cx)
                            }))
                    })),
            )
            .child(
                h_flex()
                    .gap_1()
                    .children(events.into_iter().map(|(event, label)| {
                        Button::new(format!("trigger-event-{label}"), label)
                            .size(ButtonSize::Compact)
                            .toggle_state(fields.event == event)
                            .disabled(
                                self.operation == FormOperation::Executing
                                    || !trigger_event_supported(db_type, event),
                            )
                            .on_click(
                                cx.listener(move |form, _, _, cx| {
                                    form.set_trigger_event(event, cx)
                                }),
                            )
                    })),
            )
            .into_any_element()
    }
}

impl Render for ObjectMutationForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.operation == FormOperation::Executing;
        let kind = kind_label(self.state.kind(), self.language_setting);
        let (title, description) = match &self.state {
            ObjectMutationFormState::Create { target, .. } => (
                format!("{} {kind}", text(self.language_setting, "新建", "Create")),
                format!("{} / {}", target.connection_name, target.database),
            ),
            ObjectMutationFormState::Rename {
                target,
                original_name,
                ..
            } => (
                format!("{} {kind}", text(self.language_setting, "重命名", "Rename")),
                format!(
                    "{original_name} · {} / {}",
                    target.connection_name, target.database
                ),
            ),
        };
        let fields = match &self.state {
            ObjectMutationFormState::Create { .. } => self.render_create_fields(cx),
            ObjectMutationFormState::Rename { .. } => {
                vec![self.name.clone().into_any_element()]
            }
        };

        div()
            .tab_group()
            .track_focus(&self.focus_handle(cx))
            .elevation_3(cx)
            .occlude()
            .w(rems(46.0))
            .max_h(rems(46.0))
            .child(
                Modal::new("object-mutation-form", Some(self.scroll_handle.clone()))
                    .header(
                        ModalHeader::new()
                            .headline(title)
                            .description(description)
                            .show_dismiss_button(!busy),
                    )
                    .section(
                        Section::new()
                            .child(v_flex().gap_3().children(fields))
                            .when_some(self.error.clone(), |section, error| {
                                section.child(
                                    Label::new(error)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Error)
                                        .line_clamp(4),
                                )
                            }),
                    )
                    .footer(
                        ModalFooter::new().end_slot(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new(
                                        "cancel-object-mutation",
                                        text(self.language_setting, "取消", "Cancel"),
                                    )
                                    .disabled(busy)
                                    .on_click(cx.listener(Self::cancel)),
                                )
                                .child(
                                    Button::new(
                                        "submit-object-mutation",
                                        text(self.language_setting, "执行", "Execute"),
                                    )
                                    .style(ButtonStyle::Filled)
                                    .layer(ElevationIndex::ModalSurface)
                                    .loading(busy)
                                    .disabled(busy)
                                    .on_click(
                                        cx.listener(|form, _, window, cx| form.submit(window, cx)),
                                    ),
                                ),
                        ),
                    ),
            )
    }
}

impl Focusable for ObjectMutationForm {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.name.read(cx).focus_handle(cx)
    }
}

impl ModalView for ObjectMutationForm {
    fn fade_out_background(&self) -> bool {
        true
    }

    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut Context<Self>) -> DismissDecision {
        DismissDecision::Dismiss(self.operation == FormOperation::Idle)
    }
}

fn arguments_input(
    language: UiLanguage,
    window: &mut Window,
    cx: &mut Context<ObjectMutationForm>,
) -> Entity<InputField> {
    input(
        window,
        cx,
        text(language, "例如 account_id uuid", "e.g. account_id uuid"),
        text(language, "参数", "Arguments"),
        2,
    )
}

fn return_type_input(
    default_value: &str,
    language: UiLanguage,
    window: &mut Window,
    cx: &mut Context<ObjectMutationForm>,
) -> Entity<InputField> {
    let field = input(
        window,
        cx,
        text(language, "返回类型", "Return type"),
        text(language, "返回类型", "Return type"),
        3,
    );
    set_text(&field, default_value, window, cx);
    field
}

fn routine_language_input(
    default_value: &str,
    language: UiLanguage,
    window: &mut Window,
    cx: &mut Context<ObjectMutationForm>,
) -> Entity<InputField> {
    let field = input(
        window,
        cx,
        text(language, "过程语言", "Routine language"),
        text(language, "语言", "Language"),
        4,
    );
    set_text(&field, default_value, window, cx);
    field
}

fn definition_editor(window: &mut Window, cx: &mut Context<ObjectMutationForm>) -> Entity<Editor> {
    cx.new(|cx| sql_language::editor("", window, cx))
}

fn column_draft(
    next_column_id: &mut u64,
    language: UiLanguage,
    window: &mut Window,
    cx: &mut Context<ObjectMutationForm>,
) -> ColumnDraft {
    *next_column_id = next_column_id.wrapping_add(1);
    let id = *next_column_id;
    ColumnDraft {
        id,
        name: input(
            window,
            cx,
            text(language, "列名", "Column name"),
            text(language, "列", "Column"),
            10 + id as isize * 3,
        ),
        data_type: input(
            window,
            cx,
            text(language, "数据类型", "Data type"),
            text(language, "类型", "Type"),
            11 + id as isize * 3,
        ),
        default_value: input(
            window,
            cx,
            text(language, "可选 SQL 表达式", "Optional SQL expression"),
            text(language, "默认值", "Default"),
            12 + id as isize * 3,
        ),
        nullable: true,
        primary_key: false,
    }
}

fn input(
    window: &mut Window,
    cx: &mut Context<ObjectMutationForm>,
    placeholder: &str,
    label: &str,
    tab_index: isize,
) -> Entity<InputField> {
    cx.new(|cx| {
        InputField::new(window, cx, placeholder)
            .label(label)
            .tab_index(tab_index)
    })
}

fn set_text(
    field: &Entity<InputField>,
    value: &str,
    window: &mut Window,
    cx: &mut Context<ObjectMutationForm>,
) {
    field.update(cx, |field, cx| field.set_text(value, window, cx));
}

fn field_text(field: &Entity<InputField>, cx: &App) -> String {
    field.read(cx).text(cx)
}

fn optional_text(field: &Entity<InputField>, cx: &App) -> Option<String> {
    let value = field_text(field, cx).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn qualify_name(schema: Option<&str>, name: &str) -> String {
    match schema.filter(|_| !name.contains('.')) {
        Some(schema) => format!("{schema}.{name}"),
        None => name.to_string(),
    }
}

fn rename_default_name(original_name: &str) -> &str {
    original_name
        .split_once('(')
        .map_or(original_name, |(name, _)| name)
        .rsplit('.')
        .next()
        .unwrap_or(original_name)
}

fn toggle_state(selected: bool) -> ToggleState {
    if selected {
        ToggleState::Selected
    } else {
        ToggleState::Unselected
    }
}

pub(super) fn kind_label(kind: DatabaseObjectKind, language: UiLanguage) -> &'static str {
    match kind {
        DatabaseObjectKind::Database => text(language, "数据库", "Database"),
        DatabaseObjectKind::Schema => text(language, "Schema", "Schema"),
        DatabaseObjectKind::Table => text(language, "表", "Table"),
        DatabaseObjectKind::View => text(language, "视图", "View"),
        DatabaseObjectKind::Function => text(language, "函数", "Function"),
        DatabaseObjectKind::Procedure => text(language, "存储过程", "Procedure"),
        DatabaseObjectKind::Trigger => text(language, "触发器", "Trigger"),
        DatabaseObjectKind::User => text(language, "用户", "User"),
    }
}

#[cfg(test)]
mod tests;
