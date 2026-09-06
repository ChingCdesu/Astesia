use super::super::catalog_tree::{column_row, CatalogDetail};
use super::*;

impl ConnectionProfilesPanel {
    pub(in crate::ui::connections) fn append_detail_rows(
        &self,
        key: &CatalogTableKey,
        db_type: crate::db::DbType,
        depth: usize,
        rows: &mut Vec<SidebarRow>,
    ) {
        let Some(CatalogDetail::Ready(detail)) = self.table_details.get(key) else {
            if let Some(CatalogDetail::Failed(error)) = self.table_details.get(key) {
                Self::append_message(
                    rows,
                    format!("detail-state-{key:?}"),
                    depth,
                    Some(error.clone()),
                );
            }
            return;
        };
        let columns = detail
            .columns
            .iter()
            .map(|column| {
                (
                    column.name.clone(),
                    format!(
                        "{}{}",
                        super::super::presentation::compact_column_type(db_type, &column.data_type),
                        if column.is_primary_key { " · PK" } else { "" }
                    ),
                )
            })
            .collect();
        let mut constraints = detail.constraints.as_ref().map(|items| {
            items
                .iter()
                .map(|item| {
                    (
                        item.name.clone(),
                        match item.kind {
                            crate::db::ConstraintKind::PrimaryKey => "PRIMARY KEY",
                            crate::db::ConstraintKind::Unique => "UNIQUE",
                            crate::db::ConstraintKind::Check => "CHECK",
                        }
                        .to_string(),
                    )
                })
                .collect::<Vec<_>>()
        });
        if let Some(keys) = &detail.foreign_keys {
            constraints
                .get_or_insert_with(Vec::new)
                .extend(keys.iter().map(|key| {
                    (
                        key.name.clone(),
                        format!(
                            "FOREIGN KEY ({}) → {} ({})",
                            key.from_columns.join(", "),
                            key.to_table,
                            key.to_columns.join(", ")
                        ),
                    )
                }));
        }
        let indexes = detail
            .indexes
            .iter()
            .map(|item| (item.name.clone(), item.columns.join(", ")))
            .collect();
        for (part, icon, values) in [
            (0, "column", Some(columns)),
            (1, "constraint", constraints),
            (2, "index", Some(indexes)),
        ] {
            let Some(values): Option<Vec<(String, String)>> = values else {
                continue;
            };
            let section_key = (key.clone(), part);
            let expanded = !self.collapsed_details.contains(&section_key);
            rows.push(
                SidebarRow::new(
                    format!("detail-section-{section_key:?}"),
                    depth,
                    move |panel, cx| {
                        let language = panel.settings.read(cx).language();
                        let label = match part {
                            0 => text(language, "列", "Columns"),
                            1 => text(language, "约束", "Constraints"),
                            _ => text(language, "索引", "Indexes"),
                        };
                        let click = section_key.clone();
                        let keyboard = section_key.clone();
                        tree_row(
                            format!("detail-section-{section_key:?}"),
                            label.to_string(),
                            icon,
                            Some(expanded),
                            cx,
                        )
                        .on_click(
                            cx.listener(move |panel, event: &gpui_kit::ClickEvent, _, cx| {
                                if event.click_count() > 1 {
                                    return;
                                }
                                panel.toggle_detail_section(click.clone(), cx)
                            }),
                        )
                        .on_action(cx.listener(move |panel, _: &menu::Confirm, _, cx| {
                            panel.toggle_detail_section(keyboard.clone(), cx)
                        }))
                        .into_any_element()
                    },
                )
                .highlight(false),
            );
            if expanded {
                for (index, (name, detail)) in values.into_iter().enumerate() {
                    let id = format!("detail-value-{key:?}-{part}-{index}");
                    rows.push(
                        SidebarRow::new(id.clone(), depth + 1, move |_, cx| {
                            let label = format!("{name} {detail}");
                            let row = if part == 0 {
                                column_row(id.clone(), name.clone(), detail.clone(), cx)
                            } else {
                                tree_row(id.clone(), label.clone(), icon, None, cx)
                            };
                            row.into_any_element()
                        })
                        .highlight(false),
                    );
                }
            }
        }
    }
}
