use std::sync::Arc;

use gpui::{ClickEvent, Entity, FocusHandle, Subscription};
use zed_ui::prelude::*;

use crate::application::{
    Application, QueryTarget, TableStructureLoadError, TableStructureSnapshot, TableStructureState,
    TableStructureStatus,
};
use crate::db::{ColumnInfo, ConstraintInfo, ConstraintKind, ForeignKeyInfo, IndexInfo, TableRef};

use super::localization::text;
use super::shell::ShellSettings;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StructureSection {
    Columns,
    Indexes,
    Constraints,
    ForeignKeys,
}

pub(super) struct TableStructureItem {
    application: Arc<Application>,
    state: TableStructureState,
    section: StructureSection,
    focus_handle: FocusHandle,
    settings: Entity<ShellSettings>,
    _settings_observation: Subscription,
}

impl TableStructureItem {
    pub(super) fn new(
        application: Arc<Application>,
        target: QueryTarget,
        table: TableRef,
        settings: Entity<ShellSettings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        let mut item = Self {
            application,
            state: TableStructureState::new(target, table),
            section: StructureSection::Columns,
            focus_handle: cx.focus_handle(),
            settings,
            _settings_observation: settings_observation,
        };
        item.load(cx);
        item
    }

    pub(super) fn label(&self) -> String {
        format!(
            "{} · {}/{}",
            self.state.table(),
            self.state.target().connection_name,
            self.state.target().database,
        )
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
    }

    pub(super) fn invalidate_session(
        &mut self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<Self>,
    ) {
        let language = self.settings.read(cx).language();
        if self.state.invalidate_session(
            connection_id,
            session_generation,
            text(
                language,
                "连接会话已更改。请从侧边栏重新打开表结构。",
                "The connection session changed. Reopen the table structure from the sidebar.",
            ),
        ) {
            cx.notify();
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self.state.begin_load() else {
            return;
        };
        cx.notify();

        let application = self.application.clone();
        let connection_id = self.state.target().connection_id.clone();
        let database = self.state.target().database.clone();
        let table = self.state.table().clone();
        let load = gpui_tokio::Tokio::spawn(cx, async move {
            application
                .catalog()
                .table_structure(&connection_id, &database, &table)
                .await
        });
        cx.spawn(async move |item, cx| {
            let result = match load.await {
                Ok(result) => result,
                Err(error) => Err(TableStructureLoadError::BackgroundTask(error.to_string())),
            };
            item.update(cx, |item, cx| {
                if item.state.finish_load(request, result) {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn refresh(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.load(cx);
    }

    fn show_columns(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.show_section(StructureSection::Columns, cx);
    }

    fn show_indexes(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.show_section(StructureSection::Indexes, cx);
    }

    fn show_constraints(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.show_section(StructureSection::Constraints, cx);
    }

    fn show_foreign_keys(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.show_section(StructureSection::ForeignKeys, cx);
    }

    fn show_section(&mut self, section: StructureSection, cx: &mut Context<Self>) {
        if self.section != section {
            self.section = section;
            cx.notify();
        }
    }

    fn render_columns(&self, columns: &[ColumnInfo], cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let language = self.settings.read(cx).language();
        if columns.is_empty() {
            return empty_metadata_state(
                text(language, "未发现列", "No columns found"),
                text(
                    language,
                    "此数据库未返回该表的列元数据。",
                    "This database returned no column metadata for the table.",
                ),
            );
        }
        let grid_width = px(1044.0);
        let header = h_flex()
            .id("table-structure-column-header")
            .role(gpui::Role::Row)
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.panel_background)
            .child(structure_header_cell(
                "column-position-header",
                "#",
                44.0,
                true,
            ))
            .child(structure_header_cell(
                "column-name-header",
                text(language, "列名", "Column"),
                200.0,
                false,
            ))
            .child(structure_header_cell(
                "column-type-header",
                text(language, "类型", "Type"),
                160.0,
                false,
            ))
            .child(structure_header_cell(
                "column-nullable-header",
                text(language, "可空", "Nullable"),
                80.0,
                true,
            ))
            .child(structure_header_cell(
                "column-primary-header",
                text(language, "主键", "Primary"),
                80.0,
                true,
            ))
            .child(structure_header_cell(
                "column-default-header",
                text(language, "默认值", "Default"),
                220.0,
                false,
            ))
            .child(structure_header_cell(
                "column-comment-header",
                text(language, "备注", "Comment"),
                260.0,
                false,
            ));
        let rows = div()
            .id("table-structure-column-rows")
            .role(gpui::Role::RowGroup)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                v_flex().children(columns.iter().enumerate().map(|(index, column)| {
                    h_flex()
                        .id(("table-structure-column-row", index))
                        .role(gpui::Role::Row)
                        .w_full()
                        .flex_none()
                        .border_b_1()
                        .border_color(colors.border)
                        .when(index % 2 == 1, |element| {
                            element.bg(colors.element_background)
                        })
                        .hover(|element| element.bg(colors.ghost_element_hover))
                        .child(structure_cell(
                            "column-position",
                            (index + 1).to_string(),
                            44.0,
                            true,
                        ))
                        .child(structure_cell(
                            "column-name",
                            column.name.clone(),
                            200.0,
                            false,
                        ))
                        .child(structure_cell(
                            "column-type",
                            column.data_type.clone(),
                            160.0,
                            false,
                        ))
                        .child(structure_cell(
                            "column-nullable",
                            if column.nullable {
                                text(language, "是", "Yes")
                            } else {
                                text(language, "否", "No")
                            },
                            80.0,
                            true,
                        ))
                        .child(structure_cell(
                            "column-primary",
                            if column.is_primary_key { "PK" } else { "—" },
                            80.0,
                            true,
                        ))
                        .child(structure_cell(
                            "column-default",
                            column.default_value.as_deref().unwrap_or("—"),
                            220.0,
                            false,
                        ))
                        .child(structure_cell(
                            "column-comment",
                            column.comment.as_deref().unwrap_or("—"),
                            260.0,
                            false,
                        ))
                })),
            );

        div()
            .id("table-structure-columns")
            .role(gpui::Role::Table)
            .aria_label(text(language, "表列", "Table columns"))
            .size_full()
            .overflow_x_scroll()
            .child(
                v_flex()
                    .w(grid_width)
                    .min_w_full()
                    .h_full()
                    .child(header)
                    .child(rows),
            )
            .into_any_element()
    }

    fn render_indexes(&self, indexes: &[IndexInfo], cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let language = self.settings.read(cx).language();
        if indexes.is_empty() {
            return empty_metadata_state(
                text(language, "未发现索引", "No indexes found"),
                text(
                    language,
                    "此数据库未返回该表的索引元数据。",
                    "This database returned no index metadata for the table.",
                ),
            );
        }

        let grid_width = px(884.0);
        let header = h_flex()
            .id("table-structure-index-header")
            .role(gpui::Role::Row)
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.panel_background)
            .child(structure_header_cell(
                "index-position-header",
                "#",
                44.0,
                true,
            ))
            .child(structure_header_cell(
                "index-name-header",
                text(language, "索引名", "Index"),
                260.0,
                false,
            ))
            .child(structure_header_cell(
                "index-columns-header",
                text(language, "列", "Columns"),
                420.0,
                false,
            ))
            .child(structure_header_cell(
                "index-unique-header",
                text(language, "唯一", "Unique"),
                80.0,
                true,
            ))
            .child(structure_header_cell(
                "index-primary-header",
                text(language, "主键", "Primary"),
                80.0,
                true,
            ));
        let rows = div()
            .id("table-structure-index-rows")
            .role(gpui::Role::RowGroup)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                v_flex().children(indexes.iter().enumerate().map(|(index, item)| {
                    h_flex()
                        .id(("table-structure-index-row", index))
                        .role(gpui::Role::Row)
                        .w_full()
                        .flex_none()
                        .border_b_1()
                        .border_color(colors.border)
                        .when(index % 2 == 1, |element| {
                            element.bg(colors.element_background)
                        })
                        .hover(|element| element.bg(colors.ghost_element_hover))
                        .child(structure_cell(
                            "index-position",
                            (index + 1).to_string(),
                            44.0,
                            true,
                        ))
                        .child(structure_cell(
                            "index-name",
                            item.name.clone(),
                            260.0,
                            false,
                        ))
                        .child(structure_cell(
                            "index-columns",
                            item.columns.join(", "),
                            420.0,
                            false,
                        ))
                        .child(structure_cell(
                            "index-unique",
                            if item.is_unique { "UNI" } else { "—" },
                            80.0,
                            true,
                        ))
                        .child(structure_cell(
                            "index-primary",
                            if item.is_primary { "PK" } else { "—" },
                            80.0,
                            true,
                        ))
                })),
            );

        div()
            .id("table-structure-indexes")
            .role(gpui::Role::Table)
            .aria_label(text(language, "表索引", "Table indexes"))
            .size_full()
            .overflow_x_scroll()
            .child(
                v_flex()
                    .w(grid_width)
                    .min_w_full()
                    .h_full()
                    .child(header)
                    .child(rows),
            )
            .into_any_element()
    }

    fn render_constraints(
        &self,
        constraints: &[ConstraintInfo],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors();
        let language = self.settings.read(cx).language();
        if constraints.is_empty() {
            return empty_metadata_state(
                text(language, "未发现约束", "No constraints found"),
                text(
                    language,
                    "此数据库未返回该表的约束元数据。",
                    "This database returned no constraint metadata for the table.",
                ),
            );
        }
        let header = h_flex()
            .id("table-structure-constraint-header")
            .role(gpui::Role::Row)
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.panel_background)
            .child(structure_header_cell(
                "constraint-position-header",
                "#",
                44.0,
                true,
            ))
            .child(structure_header_cell(
                "constraint-name-header",
                text(language, "约束", "Constraint"),
                280.0,
                false,
            ))
            .child(structure_header_cell(
                "constraint-type-header",
                text(language, "类型", "Type"),
                160.0,
                false,
            ))
            .child(structure_header_cell(
                "constraint-columns-header",
                text(language, "列", "Columns"),
                420.0,
                false,
            ));
        let rows = v_flex()
            .id("table-structure-constraint-rows")
            .role(gpui::Role::RowGroup)
            .children(constraints.iter().enumerate().map(|(index, item)| {
                h_flex()
                    .id(("table-structure-constraint-row", index))
                    .role(gpui::Role::Row)
                    .w_full()
                    .flex_none()
                    .border_b_1()
                    .border_color(colors.border)
                    .when(index % 2 == 1, |element| {
                        element.bg(colors.element_background)
                    })
                    .child(structure_cell(
                        "constraint-position",
                        (index + 1).to_string(),
                        44.0,
                        true,
                    ))
                    .child(structure_cell(
                        "constraint-name",
                        item.name.clone(),
                        280.0,
                        false,
                    ))
                    .child(structure_cell(
                        "constraint-type",
                        match item.kind {
                            ConstraintKind::PrimaryKey => text(language, "主键", "Primary key"),
                            ConstraintKind::Unique => text(language, "唯一", "Unique"),
                            ConstraintKind::Check => text(language, "检查", "Check"),
                        },
                        160.0,
                        false,
                    ))
                    .child(structure_cell(
                        "constraint-columns",
                        if item.columns.is_empty() {
                            item.definition.as_deref().unwrap_or("—").to_string()
                        } else {
                            item.columns.join(", ")
                        },
                        420.0,
                        false,
                    ))
            }));
        div()
            .id("table-structure-constraints")
            .role(gpui::Role::Table)
            .aria_label(text(language, "表约束", "Table constraints"))
            .size_full()
            .overflow_scroll()
            .child(v_flex().w(px(904.0)).min_w_full().child(header).child(rows))
            .into_any_element()
    }

    fn render_foreign_keys(
        &self,
        foreign_keys: &[ForeignKeyInfo],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors();
        let language = self.settings.read(cx).language();
        if foreign_keys.is_empty() {
            return empty_metadata_state(
                text(language, "未发现外键", "No foreign keys found"),
                text(
                    language,
                    "此数据库未返回该表的外键元数据。",
                    "This database returned no foreign-key metadata for the table.",
                ),
            );
        }
        let header = h_flex()
            .id("table-structure-foreign-key-header")
            .role(gpui::Role::Row)
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.panel_background)
            .child(structure_header_cell(
                "foreign-key-position-header",
                "#",
                44.0,
                true,
            ))
            .child(structure_header_cell(
                "foreign-key-name-header",
                text(language, "外键", "Foreign key"),
                240.0,
                false,
            ))
            .child(structure_header_cell(
                "foreign-key-from-header",
                text(language, "源列", "From columns"),
                260.0,
                false,
            ))
            .child(structure_header_cell(
                "foreign-key-table-header",
                text(language, "目标表", "Target table"),
                260.0,
                false,
            ))
            .child(structure_header_cell(
                "foreign-key-to-header",
                text(language, "目标列", "Target columns"),
                260.0,
                false,
            ));
        let rows = v_flex()
            .id("table-structure-foreign-key-rows")
            .role(gpui::Role::RowGroup)
            .children(foreign_keys.iter().enumerate().map(|(index, item)| {
                h_flex()
                    .id(("table-structure-foreign-key-row", index))
                    .role(gpui::Role::Row)
                    .w_full()
                    .flex_none()
                    .border_b_1()
                    .border_color(colors.border)
                    .when(index % 2 == 1, |element| {
                        element.bg(colors.element_background)
                    })
                    .child(structure_cell(
                        "foreign-key-position",
                        (index + 1).to_string(),
                        44.0,
                        true,
                    ))
                    .child(structure_cell(
                        "foreign-key-name",
                        item.name.clone(),
                        240.0,
                        false,
                    ))
                    .child(structure_cell(
                        "foreign-key-from",
                        item.from_columns.join(", "),
                        260.0,
                        false,
                    ))
                    .child(structure_cell(
                        "foreign-key-table",
                        item.to_table.to_string(),
                        260.0,
                        false,
                    ))
                    .child(structure_cell(
                        "foreign-key-to",
                        item.to_columns.join(", "),
                        260.0,
                        false,
                    ))
            }));
        div()
            .id("table-structure-foreign-keys")
            .role(gpui::Role::Table)
            .aria_label(text(language, "表外键", "Table foreign keys"))
            .size_full()
            .overflow_scroll()
            .child(
                v_flex()
                    .w(px(1064.0))
                    .min_w_full()
                    .child(header)
                    .child(rows),
            )
            .into_any_element()
    }

    fn render_ready(
        &self,
        snapshot: &TableStructureSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.section {
            StructureSection::Columns => self.render_columns(&snapshot.columns, cx),
            StructureSection::Indexes => self.render_indexes(&snapshot.indexes, cx),
            StructureSection::Constraints => snapshot
                .constraints
                .as_deref()
                .map(|constraints| self.render_constraints(constraints, cx))
                .unwrap_or_else(|| div().into_any_element()),
            StructureSection::ForeignKeys => snapshot
                .foreign_keys
                .as_deref()
                .map(|foreign_keys| self.render_foreign_keys(foreign_keys, cx))
                .unwrap_or_else(|| div().into_any_element()),
        }
    }
}

impl Render for TableStructureItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let language = self.settings.read(cx).language();
        let status = self.state.status();
        let loading = matches!(status, TableStructureStatus::Loading);
        let label = self.label();
        let target_label = format!(
            "{} / {}",
            self.state.target().connection_name,
            self.state.target().database
        );
        let counts = match status {
            TableStructureStatus::Ready(snapshot) => Some((
                snapshot.columns.len(),
                snapshot.indexes.len(),
                snapshot.constraints.as_ref().map(Vec::len),
                snapshot.foreign_keys.as_ref().map(Vec::len),
            )),
            _ => None,
        };
        let content = match status {
            TableStructureStatus::Idle | TableStructureStatus::Loading => v_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .gap_1()
                .child(
                    Label::new(text(
                        language,
                        "正在加载表结构…",
                        "Loading table structure…",
                    ))
                    .size(LabelSize::Small),
                )
                .child(
                    Label::new(label.clone())
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .into_any_element(),
            TableStructureStatus::Failed(error) => v_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .gap_2()
                .p_6()
                .text_center()
                .child(
                    Label::new(text(
                        language,
                        "无法加载表结构",
                        "Could not load table structure",
                    ))
                    .size(LabelSize::Small)
                    .color(Color::Error),
                )
                .child(
                    Label::new(localized_load_error(error, language))
                        .size(LabelSize::XSmall)
                        .line_clamp(4),
                )
                .child(
                    Button::new("retry-table-structure", text(language, "重试", "Retry"))
                        .size(ButtonSize::Compact)
                        .on_click(cx.listener(Self::refresh)),
                )
                .into_any_element(),
            TableStructureStatus::Unavailable(reason) => v_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .gap_2()
                .p_6()
                .text_center()
                .child(
                    Label::new(text(
                        language,
                        "表结构已失效",
                        "Table structure is no longer live",
                    ))
                    .size(LabelSize::Small)
                    .color(Color::Warning),
                )
                .child(
                    Label::new(reason.to_string())
                        .size(LabelSize::XSmall)
                        .line_clamp(4),
                )
                .into_any_element(),
            TableStructureStatus::Ready(snapshot) => self.render_ready(snapshot, cx),
        };

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("TableStructureItem")
            .size_full()
            .border_1()
            .border_color(colors.border.opacity(0.0))
            .focus_visible(|element| element.border_color(colors.border_focused))
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
                        h_flex()
                            .min_w_0()
                            .flex_1()
                            .gap_2()
                            .child(
                                div().max_w(px(360.0)).child(
                                    Label::new(label)
                                        .size(LabelSize::Small)
                                        .weight(gpui::FontWeight::MEDIUM)
                                        .truncate(),
                                ),
                            )
                            .child(
                                Label::new(target_label)
                                    .flex_1()
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                                    .truncate(),
                            ),
                    )
                    .child(
                        Button::new("refresh-table-structure", text(language, "刷新", "Refresh"))
                            .size(ButtonSize::Compact)
                            .loading(loading)
                            .disabled(
                                loading || matches!(status, TableStructureStatus::Unavailable(_)),
                            )
                            .on_click(cx.listener(Self::refresh)),
                    ),
            )
            .when_some(
                counts,
                |element, (column_count, index_count, constraint_count, foreign_key_count)| {
                    element.child(
                        h_flex()
                            .h(px(34.0))
                            .flex_none()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .border_b_1()
                            .border_color(colors.border)
                            .bg(colors.panel_background)
                            .child(
                                Button::new(
                                    "show-table-columns",
                                    format!("{} ({column_count})", text(language, "列", "Columns")),
                                )
                                .size(ButtonSize::Compact)
                                .toggle_state(self.section == StructureSection::Columns)
                                .style(if self.section == StructureSection::Columns {
                                    ButtonStyle::Filled
                                } else {
                                    ButtonStyle::Transparent
                                })
                                .on_click(cx.listener(Self::show_columns)),
                            )
                            .child(
                                Button::new(
                                    "show-table-indexes",
                                    format!(
                                        "{} ({index_count})",
                                        text(language, "索引", "Indexes")
                                    ),
                                )
                                .size(ButtonSize::Compact)
                                .toggle_state(self.section == StructureSection::Indexes)
                                .style(if self.section == StructureSection::Indexes {
                                    ButtonStyle::Filled
                                } else {
                                    ButtonStyle::Transparent
                                })
                                .on_click(cx.listener(Self::show_indexes)),
                            )
                            .when_some(constraint_count, |element, constraint_count| {
                                element.child(
                                    Button::new(
                                        "show-table-constraints",
                                        format!(
                                            "{} ({constraint_count})",
                                            text(language, "约束", "Constraints")
                                        ),
                                    )
                                    .size(ButtonSize::Compact)
                                    .toggle_state(self.section == StructureSection::Constraints)
                                    .style(if self.section == StructureSection::Constraints {
                                        ButtonStyle::Filled
                                    } else {
                                        ButtonStyle::Transparent
                                    })
                                    .on_click(cx.listener(Self::show_constraints)),
                                )
                            })
                            .when_some(foreign_key_count, |element, foreign_key_count| {
                                element.child(
                                    Button::new(
                                        "show-table-foreign-keys",
                                        format!(
                                            "{} ({foreign_key_count})",
                                            text(language, "外键", "Foreign Keys")
                                        ),
                                    )
                                    .size(ButtonSize::Compact)
                                    .toggle_state(self.section == StructureSection::ForeignKeys)
                                    .style(if self.section == StructureSection::ForeignKeys {
                                        ButtonStyle::Filled
                                    } else {
                                        ButtonStyle::Transparent
                                    })
                                    .on_click(cx.listener(Self::show_foreign_keys)),
                                )
                            }),
                    )
                },
            )
            .child(content)
    }
}

fn structure_cell(
    id: &'static str,
    value: impl Into<SharedString>,
    width: f32,
    centered: bool,
) -> AnyElement {
    div()
        .id(id)
        .role(gpui::Role::Cell)
        .w(px(width))
        .flex_none()
        .px_2()
        .py_1()
        .when(centered, |element| element.text_center())
        .child(Label::new(value).size(LabelSize::XSmall).truncate())
        .into_any_element()
}

fn structure_header_cell(
    id: &'static str,
    value: impl Into<SharedString>,
    width: f32,
    centered: bool,
) -> AnyElement {
    div()
        .id(id)
        .role(gpui::Role::ColumnHeader)
        .w(px(width))
        .flex_none()
        .px_2()
        .py_1()
        .when(centered, |element| element.text_center())
        .child(Label::new(value).size(LabelSize::XSmall).truncate())
        .into_any_element()
}

fn empty_metadata_state(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
) -> AnyElement {
    v_flex()
        .size_full()
        .justify_center()
        .items_center()
        .gap_1()
        .child(Label::new(title).size(LabelSize::Small))
        .child(
            Label::new(description)
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .into_any_element()
}

fn localized_load_error(
    error: &TableStructureLoadError,
    language: crate::platform::UiLanguage,
) -> String {
    let stage = match error {
        TableStructureLoadError::Connection(_) => text(language, "连接", "Connection"),
        TableStructureLoadError::Unsupported(_) => text(language, "不支持的操作", "Unsupported"),
        TableStructureLoadError::Columns(_) => text(language, "加载列", "Loading columns"),
        TableStructureLoadError::Indexes(_) => text(language, "加载索引", "Loading indexes"),
        TableStructureLoadError::Constraints(_) => {
            text(language, "加载约束", "Loading constraints")
        }
        TableStructureLoadError::ForeignKeys(_) => {
            text(language, "加载外键", "Loading foreign keys")
        }
        TableStructureLoadError::BackgroundTask(_) => text(
            language,
            "表结构后台任务",
            "Table-structure background task",
        ),
    };
    format!("{stage}: {}", error.message())
}
