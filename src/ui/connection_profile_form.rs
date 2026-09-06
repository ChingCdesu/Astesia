mod input;
#[cfg(test)]
mod tests;
mod view;

use std::sync::Arc;

use crate::ui::components::prelude::*;
use gpui_kit::component::input::InputEvent as ErasedEditorEvent;
use gpui_kit::{
    actions, App, Context, DismissEvent, Entity, EventEmitter, ScrollHandle, Subscription, Window,
};
use input::InputField;
use uuid::Uuid;

use crate::application::{
    Application, ConnectionOutcome, ProfileDraft, ProfileDraftField, ProfileOrigin,
    ValidatedProfile,
};
use crate::connection_repository::{ConnectionRepositoryError, SharedConnectionProfile};
use crate::db::DbType;
use crate::platform::UiLanguage;

use super::engine_presentation::engine_hex_color;
use super::localization::text;

actions!(astesia, [SubmitConnectionProfile]);

pub(super) fn bind_connection_profile_form_keys(cx: &mut App) {
    cx.bind_keys([
        gpui_kit::KeyBinding::new(
            "cmd-enter",
            SubmitConnectionProfile,
            Some("ConnectionProfileForm"),
        ),
        gpui_kit::KeyBinding::new(
            "ctrl-enter",
            SubmitConnectionProfile,
            Some("ConnectionProfileForm"),
        ),
        gpui_kit::KeyBinding::new("escape", menu::Cancel, Some("ConnectionProfileForm")),
        gpui_kit::KeyBinding::new(
            "tab",
            menu::SelectNext,
            Some("ConnectionProfileForm > Input"),
        ),
        gpui_kit::KeyBinding::new(
            "shift-tab",
            menu::SelectPrevious,
            Some("ConnectionProfileForm > Input"),
        ),
        gpui_kit::KeyBinding::new("tab", menu::SelectNext, Some("ConnectionProfileForm")),
        gpui_kit::KeyBinding::new(
            "shift-tab",
            menu::SelectPrevious,
            Some("ConnectionProfileForm"),
        ),
        gpui_kit::KeyBinding::new("enter", menu::Confirm, Some("ConnectionProfileFormControl")),
        gpui_kit::KeyBinding::new("space", menu::Confirm, Some("ConnectionProfileFormControl")),
    ]);
}

#[derive(Clone)]
pub(super) enum ConnectionProfileFormMode {
    Create,
    Edit(Arc<SharedConnectionProfile>),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FormOperation {
    #[default]
    Idle,
    Testing,
    Saving,
}

impl FormOperation {
    const fn is_busy(self) -> bool {
        !matches!(self, Self::Idle)
    }
}

pub(super) struct ConnectionProfileSaved {
    pub(super) profile: SharedConnectionProfile,
}

#[derive(Clone, Copy)]
enum NoticeKind {
    Info,
    Success,
    Warning,
    Error,
}

struct FormNotice {
    kind: NoticeKind,
    message: String,
    detail: Option<String>,
}

struct FormFields {
    name: Entity<InputField>,
    endpoint: Entity<InputField>,
    port: Entity<InputField>,
    username: Entity<InputField>,
    password: Entity<InputField>,
    database: Entity<InputField>,
    group_name: Entity<InputField>,
    tags: Entity<InputField>,
    color: Entity<InputField>,
}

impl FormFields {
    fn all(&self) -> [&Entity<InputField>; 9] {
        [
            &self.name,
            &self.endpoint,
            &self.port,
            &self.username,
            &self.password,
            &self.database,
            &self.group_name,
            &self.tags,
            &self.color,
        ]
    }
}

struct InitialValues {
    name: String,
    endpoint: String,
    port: String,
    username: String,
    database: String,
    group_name: String,
    tags: String,
    color: String,
    has_credential: bool,
}

pub(super) struct ConnectionProfileForm {
    application: Arc<Application>,
    origin: ProfileOrigin,
    db_type: DbType,
    fields: FormFields,
    scroll_handle: ScrollHandle,
    operation: FormOperation,
    test_notice: Option<FormNotice>,
    save_notice: Option<FormNotice>,
    language: UiLanguage,
    _input_subscriptions: Vec<Subscription>,
}

impl ConnectionProfileForm {
    pub(super) fn new(
        application: Arc<Application>,
        mode: ConnectionProfileFormMode,
        language: UiLanguage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (origin, db_type, values) = match mode {
            ConnectionProfileFormMode::Create => {
                let db_type = DbType::MySQL;
                let spec = db_type.profile_spec();
                (
                    ProfileOrigin::create(Uuid::new_v4().to_string()),
                    db_type,
                    InitialValues {
                        name: String::new(),
                        endpoint: spec.default_endpoint().to_string(),
                        port: spec.default_port().to_string(),
                        username: spec.default_username().to_string(),
                        database: spec.default_database().unwrap_or_default().to_string(),
                        group_name: String::new(),
                        tags: String::new(),
                        color: engine_hex_color(db_type).to_string(),
                        has_credential: false,
                    },
                )
            }
            ConnectionProfileFormMode::Edit(profile) => {
                let origin = ProfileOrigin::edit(&profile);
                let values = InitialValues {
                    name: profile.name.clone(),
                    endpoint: profile.host.clone(),
                    port: profile.port.to_string(),
                    username: profile.username.clone(),
                    database: profile.database.clone().unwrap_or_default(),
                    group_name: profile.group_name.clone().unwrap_or_default(),
                    tags: profile.tags.join(", "),
                    color: profile.color.clone().unwrap_or_default(),
                    has_credential: profile.has_credential,
                };
                (origin, profile.db_type, values)
            }
        };

        let fields = FormFields {
            name: input(
                window,
                cx,
                text(language, "请输入连接名称", "Enter a connection name"),
                text(language, "连接名称", "Connection Name"),
                8,
            ),
            endpoint: code_input(
                window,
                cx,
                endpoint_placeholder(db_type, language),
                endpoint_label(db_type, language),
                9,
            ),
            port: code_input(
                window,
                cx,
                text(language, "数据库端口", "Database port"),
                text(language, "端口", "Port"),
                10,
            ),
            username: code_input(
                window,
                cx,
                text(language, "请输入用户名", "Enter a username"),
                text(language, "用户名", "Username"),
                11,
            ),
            password: cx.new(|cx| {
                InputField::password(
                    window,
                    cx,
                    if values.has_credential {
                        text(
                            language,
                            "密码已安全保存；留空将保留原密码",
                            "Password is stored securely; leave blank to keep it",
                        )
                    } else {
                        text(
                            language,
                            "请输入密码（可选）",
                            "Enter a password (optional)",
                        )
                    },
                    12,
                )
                .label(text(language, "密码", "Password"))
            }),
            database: code_input(
                window,
                cx,
                text(
                    language,
                    "请输入数据库名（可选）",
                    "Enter a database (optional)",
                ),
                text(language, "数据库 · 可选", "Database · Optional"),
                13,
            ),
            group_name: input(
                window,
                cx,
                text(language, "输入分组名称（可选）", "Enter a group (optional)"),
                text(language, "分组 · 可选", "Group · Optional"),
                14,
            ),
            tags: input(
                window,
                cx,
                text(
                    language,
                    "以逗号分隔标签（最多 20 个）",
                    "Comma-separated tags (up to 20)",
                ),
                text(language, "标签 · 可选", "Tags · Optional"),
                16,
            ),
            color: code_input(
                window,
                cx,
                text(language, "#RRGGBB（可选）", "#RRGGBB (optional)"),
                text(language, "颜色", "Color"),
                15,
            ),
        };

        set_text(&fields.name, &values.name, window, cx);
        set_text(&fields.endpoint, &values.endpoint, window, cx);
        set_text(&fields.port, &values.port, window, cx);
        set_text(&fields.username, &values.username, window, cx);
        set_text(&fields.database, &values.database, window, cx);
        set_text(&fields.group_name, &values.group_name, window, cx);
        set_text(&fields.tags, &values.tags, window, cx);
        set_text(&fields.color, &values.color, window, cx);

        let input_subscriptions = fields
            .all()
            .into_iter()
            .map(|field| {
                cx.subscribe(field, |form, _, event: &ErasedEditorEvent, cx| {
                    form.handle_input_event(event, cx);
                })
            })
            .collect();

        Self {
            application,
            origin,
            db_type,
            fields,
            scroll_handle: ScrollHandle::new(),
            operation: FormOperation::Idle,
            test_notice: None,
            save_notice: None,
            language,
            _input_subscriptions: input_subscriptions,
        }
    }

    fn handle_input_event(&mut self, event: &ErasedEditorEvent, cx: &mut Context<Self>) {
        if !matches!(event, ErasedEditorEvent::Change) {
            return;
        }
        if !self.operation.is_busy() {
            self.test_notice = None;
            self.save_notice = None;
        }
        cx.notify();
    }

    fn set_inputs_read_only(&self, read_only: bool, cx: &mut Context<Self>) {
        for field in self.fields.all() {
            field.update(cx, |field, cx| field.set_read_only(read_only, cx));
        }
    }

    fn select_db_type(&mut self, db_type: DbType, window: &mut Window, cx: &mut Context<Self>) {
        if self.operation.is_busy() || self.db_type == db_type {
            return;
        }

        self.db_type = db_type;
        self.fields.endpoint.update(cx, |field, cx| {
            field.set_label(endpoint_label(db_type, self.language), cx);
            field.set_placeholder_text(endpoint_placeholder(db_type, self.language), window, cx);
        });
        let spec = db_type.profile_spec();
        set_text(
            &self.fields.port,
            &spec.default_port().to_string(),
            window,
            cx,
        );
        set_text(&self.fields.color, engine_hex_color(db_type), window, cx);
        self.fields
            .password
            .update(cx, |input, cx| input.clear(window, cx));

        if spec.is_file() {
            if self.fields.endpoint.read(cx).text(cx).trim() == "localhost" {
                self.fields
                    .endpoint
                    .update(cx, |input, cx| input.clear(window, cx));
            }
        } else if self.fields.endpoint.read(cx).is_empty(cx) {
            set_text(&self.fields.endpoint, spec.default_endpoint(), window, cx);
        }

        if !spec.is_file() {
            if self.fields.username.read(cx).is_empty(cx) {
                set_text(&self.fields.username, spec.default_username(), window, cx);
            }
        } else {
            self.fields
                .username
                .update(cx, |input, cx| input.clear(window, cx));
        }

        if !spec.is_file() {
            if self.fields.database.read(cx).is_empty(cx) {
                if let Some(database) = spec.default_database() {
                    set_text(&self.fields.database, database, window, cx);
                }
            }
        } else {
            self.fields
                .database
                .update(cx, |input, cx| input.clear(window, cx));
        }

        self.test_notice = None;
        self.save_notice = None;
        cx.notify();
    }

    fn cancel(&mut self, _: &menu::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(cx);
    }

    fn cancel_click(&mut self, _: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(cx);
    }

    fn cancel_confirm(&mut self, _: &menu::Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(cx);
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if !self.operation.is_busy() {
            cx.emit(DismissEvent);
        }
    }

    fn submit_action(
        &mut self,
        _: &SubmitConnectionProfile,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save(cx);
    }

    fn focus_next(&mut self, _: &menu::SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_next(cx);
    }

    fn focus_previous(
        &mut self,
        _: &menu::SelectPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus_prev(cx);
    }

    fn test_click(&mut self, _: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.test(cx);
    }

    fn test_confirm(&mut self, _: &menu::Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.test(cx);
    }

    fn save_click(&mut self, _: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    fn save_confirm(&mut self, _: &menu::Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    fn test(&mut self, cx: &mut Context<Self>) {
        if self.operation.is_busy() {
            return;
        }
        let Some(request) = self.validated_request(cx) else {
            self.test_notice = Some(FormNotice::error(text(
                self.language,
                "请修正标记的字段后再测试连接",
                "Fix the marked fields before testing the connection",
            )));
            cx.notify();
            return;
        };

        self.operation = FormOperation::Testing;
        self.set_inputs_read_only(true, cx);
        self.test_notice = None;
        let application = self.application.clone();
        let task = crate::ui::runtime::spawn(cx, async move {
            application
                .connections()
                .test_connection(request.config().clone())
                .await
        });
        cx.spawn(async move |form, cx| {
            let result = task.await;
            form.update(cx, |form, cx| {
                form.operation = FormOperation::Idle;
                form.set_inputs_read_only(false, cx);
                form.test_notice = Some(match result {
                    Ok(Ok(ConnectionOutcome::Succeeded)) => FormNotice::success(
                        text(form.language, "连接成功", "Connection succeeded"),
                        form.language,
                    ),
                    Ok(Ok(ConnectionOutcome::Rejected(message))) => {
                        FormNotice::test_failed(message, form.language)
                    }
                    Ok(Err(error)) => FormNotice::test_failed(error, form.language),
                    Err(error) => FormNotice::error(format!(
                        "{}: {error}",
                        text(
                            form.language,
                            "测试连接的后台任务意外结束",
                            "Connection test task ended unexpectedly",
                        )
                    )),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.operation.is_busy() {
            return;
        }
        let Some(request) = self.validated_request(cx) else {
            self.save_notice = Some(FormNotice::error(text(
                self.language,
                "请修正标记的字段后再保存",
                "Fix the marked fields before saving",
            )));
            cx.notify();
            return;
        };

        self.operation = FormOperation::Saving;
        self.set_inputs_read_only(true, cx);
        self.save_notice = None;
        let application = self.application.clone();
        let task = crate::ui::runtime::spawn(cx, async move {
            application.connections().save_profile(request).await
        });
        cx.spawn(async move |form, cx| {
            let result = task.await;
            form.update(cx, |form, cx| {
                form.operation = FormOperation::Idle;
                form.set_inputs_read_only(false, cx);
                match result {
                    Ok(Ok(profile)) => {
                        cx.emit(ConnectionProfileSaved { profile });
                        cx.emit(DismissEvent);
                    }
                    Ok(Err(error)) => {
                        form.save_notice = Some(FormNotice::repository_error(error, form.language));
                        cx.notify();
                    }
                    Err(error) => {
                        form.save_notice = Some(FormNotice::error(format!(
                            "{}: {error}",
                            text(
                                form.language,
                                "保存连接的后台任务意外结束",
                                "Save connection task ended unexpectedly",
                            )
                        )));
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn validated_request(&mut self, cx: &mut Context<Self>) -> Option<ValidatedProfile> {
        for field in [
            &self.fields.name,
            &self.fields.endpoint,
            &self.fields.port,
            &self.fields.tags,
            &self.fields.color,
        ] {
            clear_error(field, cx);
        }

        let draft = ProfileDraft {
            db_type: self.db_type,
            name: field_text(&self.fields.name, cx),
            endpoint: field_text(&self.fields.endpoint, cx),
            port: field_text(&self.fields.port, cx),
            username: field_text(&self.fields.username, cx),
            password: field_text(&self.fields.password, cx),
            database: field_text(&self.fields.database, cx),
            group_name: field_text(&self.fields.group_name, cx),
            tags: field_text(&self.fields.tags, cx),
            color: field_text(&self.fields.color, cx),
        };
        match draft.validate(&self.origin) {
            Ok(request) => Some(request),
            Err(errors) => {
                for error in errors {
                    let field = match error.field() {
                        ProfileDraftField::Name => &self.fields.name,
                        ProfileDraftField::Endpoint => &self.fields.endpoint,
                        ProfileDraftField::Port => &self.fields.port,
                        ProfileDraftField::Tags => &self.fields.tags,
                        ProfileDraftField::Color => &self.fields.color,
                    };
                    set_error(field, validation_message(error, self.language), cx);
                }
                None
            }
        }
    }
}

impl EventEmitter<ConnectionProfileSaved> for ConnectionProfileForm {}
impl EventEmitter<DismissEvent> for ConnectionProfileForm {}

impl FormNotice {
    fn success(message: impl Into<String>, language: UiLanguage) -> Self {
        Self {
            kind: NoticeKind::Success,
            message: message.into(),
            detail: Some(
                text(
                    language,
                    "连接配置尚未保存。",
                    "The connection profile has not been saved.",
                )
                .to_string(),
            ),
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Error,
            message: message.into(),
            detail: None,
        }
    }

    fn warning(message: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Warning,
            message: message.into(),
            detail: None,
        }
    }

    fn test_failed(detail: impl Into<String>, language: UiLanguage) -> Self {
        Self {
            kind: NoticeKind::Error,
            message: text(language, "连接测试失败", "Connection test failed").to_string(),
            detail: Some(detail.into()),
        }
    }

    fn repository_error(error: ConnectionRepositoryError, language: UiLanguage) -> Self {
        let detail = if language == UiLanguage::English
            && error.code
                == crate::connection_repository::ConnectionRepositoryErrorCode::StorageBusy
        {
            "Connection storage is busy. Wait for the other operation to finish, then retry.\nError code: storage_busy".to_string()
        } else {
            format!(
                "{}\n{} {}{}",
                error.message,
                error.remediation,
                text(language, "错误码：", "Error code: "),
                error.code.as_str()
            )
        };
        Self {
            kind: NoticeKind::Error,
            message: text(language, "连接保存失败", "Could not save the connection").to_string(),
            detail: Some(detail),
        }
    }
}

fn input(
    window: &mut Window,
    cx: &mut Context<ConnectionProfileForm>,
    placeholder: &str,
    label: &str,
    tab_index: isize,
) -> Entity<InputField> {
    cx.new(|cx| {
        InputField::new(window, cx, placeholder)
            .tab_index(tab_index)
            .label(label)
    })
}

fn code_input(
    window: &mut Window,
    cx: &mut Context<ConnectionProfileForm>,
    placeholder: &str,
    label: &str,
    tab_index: isize,
) -> Entity<InputField> {
    cx.new(|cx| {
        InputField::new(window, cx, placeholder)
            .tab_index(tab_index)
            .label(label)
            .code_value()
    })
}

fn endpoint_label(db_type: DbType, language: UiLanguage) -> &'static str {
    if db_type.profile_spec().is_file() {
        text(language, "数据库文件路径", "Database File Path")
    } else {
        text(language, "主机", "Host")
    }
}

fn endpoint_placeholder(db_type: DbType, language: UiLanguage) -> &'static str {
    if db_type.profile_spec().is_file() {
        text(
            language,
            "请输入 SQLite 文件路径",
            "Enter a SQLite file path",
        )
    } else {
        text(language, "请输入主机地址", "Enter a host address")
    }
}

fn validation_message(
    error: crate::application::ProfileValidationError,
    language: UiLanguage,
) -> &'static str {
    use crate::application::ProfileValidationError::*;
    match error {
        NameRequired => text(language, "连接名称不能为空", "Connection name is required"),
        FilePathRequired => text(
            language,
            "SQLite 文件路径不能为空",
            "SQLite file path is required",
        ),
        HostRequired => text(language, "主机地址不能为空", "Host address is required"),
        InvalidPort => text(
            language,
            "端口必须是 1–65535 的整数",
            "Port must be an integer from 1 to 65535",
        ),
        InvalidColor => text(
            language,
            "颜色必须使用 #RRGGBB 格式",
            "Use #RRGGBB format for the color",
        ),
        InvalidTag => text(
            language,
            "每个标签最多 64 个字符，且不能包含控制字符",
            "Each tag allows up to 64 characters and no control characters",
        ),
        TooManyTags => text(
            language,
            "每个连接最多可设置 20 个标签",
            "Use no more than 20 tags per connection",
        ),
    }
}

fn set_text(
    field: &Entity<InputField>,
    value: &str,
    window: &mut Window,
    cx: &mut Context<ConnectionProfileForm>,
) {
    field.update(cx, |input, cx| input.set_text(value, window, cx));
}

fn field_text(field: &Entity<InputField>, cx: &Context<ConnectionProfileForm>) -> String {
    field.read(cx).text(cx)
}

fn clear_error(field: &Entity<InputField>, cx: &mut Context<ConnectionProfileForm>) {
    field.update(cx, |input, cx| input.set_error(None::<SharedString>, cx));
}

fn set_error(
    field: &Entity<InputField>,
    error: impl Into<SharedString>,
    cx: &mut Context<ConnectionProfileForm>,
) {
    field.update(cx, |input, cx| input.set_error(Some(error), cx));
}
