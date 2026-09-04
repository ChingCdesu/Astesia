use super::*;

pub(super) struct ColumnDraft {
    pub(super) id: u64,
    pub(super) name: Entity<InputField>,
    pub(super) data_type: Entity<InputField>,
    pub(super) default_value: Entity<InputField>,
    pub(super) nullable: bool,
    pub(super) primary_key: bool,
}

pub(super) struct TableFields {
    pub(super) columns: Vec<ColumnDraft>,
    pub(super) next_column_id: u64,
}

pub(super) struct DefinitionFields {
    pub(super) definition: Entity<Editor>,
}

pub(super) struct FunctionFields {
    pub(super) arguments: Entity<InputField>,
    pub(super) return_type: Entity<InputField>,
    pub(super) language: Entity<InputField>,
    pub(super) definition: Entity<Editor>,
}

pub(super) struct ProcedureFields {
    pub(super) arguments: Entity<InputField>,
    pub(super) language: Entity<InputField>,
    pub(super) definition: Entity<Editor>,
}

pub(super) struct TriggerFields {
    pub(super) table: Entity<InputField>,
    pub(super) timing: TriggerTiming,
    pub(super) event: TriggerEvent,
    pub(super) definition: Entity<Editor>,
}

pub(super) struct UserFields {
    pub(super) host: Entity<InputField>,
    pub(super) password: Entity<InputField>,
}

pub(super) enum CreateObjectDraft {
    Database,
    Schema,
    Table(TableFields),
    View(DefinitionFields),
    Function(FunctionFields),
    Procedure(ProcedureFields),
    Trigger(TriggerFields),
    User(UserFields),
}

pub(super) enum ObjectMutationFormState {
    Create {
        target: QueryTarget,
        schema: Option<String>,
        draft: CreateObjectDraft,
    },
    Rename {
        target: QueryTarget,
        kind: DatabaseObjectKind,
        original_name: String,
    },
}

impl ObjectMutationFormState {
    pub(super) fn target(&self) -> &QueryTarget {
        match self {
            Self::Create { target, .. } | Self::Rename { target, .. } => target,
        }
    }

    pub(super) fn kind(&self) -> DatabaseObjectKind {
        match self {
            Self::Create { draft, .. } => draft.kind(),
            Self::Rename { kind, .. } => *kind,
        }
    }
}

impl CreateObjectDraft {
    pub(super) fn new(
        kind: DatabaseObjectKind,
        policy: ObjectCreationPolicy,
        language: UiLanguage,
        window: &mut Window,
        cx: &mut Context<ObjectMutationForm>,
    ) -> Self {
        match kind {
            DatabaseObjectKind::Database => Self::Database,
            DatabaseObjectKind::Schema => Self::Schema,
            DatabaseObjectKind::Table => {
                let mut fields = TableFields {
                    columns: Vec::new(),
                    next_column_id: 0,
                };
                let column = column_draft(&mut fields.next_column_id, language, window, cx);
                set_text(&column.name, "id", window, cx);
                set_text(&column.data_type, policy.default_column_type, window, cx);
                fields.columns.push(column);
                Self::Table(fields)
            }
            DatabaseObjectKind::View => Self::View(DefinitionFields {
                definition: definition_editor(window, cx),
            }),
            DatabaseObjectKind::Function => Self::Function(FunctionFields {
                arguments: arguments_input(language, window, cx),
                return_type: return_type_input(policy.default_return_type, language, window, cx),
                language: routine_language_input(
                    policy.default_routine_language,
                    language,
                    window,
                    cx,
                ),
                definition: definition_editor(window, cx),
            }),
            DatabaseObjectKind::Procedure => Self::Procedure(ProcedureFields {
                arguments: arguments_input(language, window, cx),
                language: routine_language_input(
                    policy.default_routine_language,
                    language,
                    window,
                    cx,
                ),
                definition: definition_editor(window, cx),
            }),
            DatabaseObjectKind::Trigger => Self::Trigger(TriggerFields {
                table: input(
                    window,
                    cx,
                    text(language, "触发器所属表", "Trigger table"),
                    text(language, "表", "Table"),
                    5,
                ),
                timing: policy.default_trigger_timing,
                event: TriggerEvent::Insert,
                definition: definition_editor(window, cx),
            }),
            DatabaseObjectKind::User => {
                let host = input(window, cx, "%", text(language, "主机", "Host"), 6);
                set_text(&host, "%", window, cx);
                let password = cx.new(|cx| {
                    InputField::new(window, cx, text(language, "输入密码", "Enter password"))
                        .label(text(language, "密码", "Password"))
                        .tab_index(7)
                        .masked(true)
                });
                Self::User(UserFields { host, password })
            }
        }
    }

    pub(super) fn kind(&self) -> DatabaseObjectKind {
        match self {
            Self::Database => DatabaseObjectKind::Database,
            Self::Schema => DatabaseObjectKind::Schema,
            Self::Table(_) => DatabaseObjectKind::Table,
            Self::View(_) => DatabaseObjectKind::View,
            Self::Function(_) => DatabaseObjectKind::Function,
            Self::Procedure(_) => DatabaseObjectKind::Procedure,
            Self::Trigger(_) => DatabaseObjectKind::Trigger,
            Self::User(_) => DatabaseObjectKind::User,
        }
    }
}
