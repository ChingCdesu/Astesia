use super::super::catalog_tree::CatalogDetail;
use super::*;

impl ConnectionProfilesPanel {
    pub(super) fn append_detail_rows(
        &self,
        key: &CatalogTableKey,
        depth: usize,
        rows: &mut Vec<SidebarRow>,
    ) {
        let Some(CatalogDetail::Ready(detail)) = self.table_details.get(key) else {
            let error = match self.table_details.get(key) {
                Some(CatalogDetail::Failed(error)) => Some(error.clone()),
                _ => None,
            };
            Self::append_message(rows, format!("detail-state-{key:?}"), depth, error);
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
                        column.data_type,
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
        for (part, label, icon, values) in [
            (0, "Columns", "column", Some(columns)),
            (1, "Constraints", "constraint", constraints),
            (2, "Indexes", "index", Some(indexes)),
        ] {
            let Some(values): Option<Vec<(String, String)>> = values else {
                continue;
            };
            let section_key = (key.clone(), part);
            let expanded = !self.collapsed_details.contains(&section_key);
            rows.push(SidebarRow::new(
                format!("detail-section-{section_key:?}"),
                depth,
                move |_, cx| {
                    let click = section_key.clone();
                    let keyboard = section_key.clone();
                    tree_row(
                        format!("detail-section-{section_key:?}"),
                        label.to_string(),
                        icon,
                        Some(expanded),
                        cx,
                    )
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        panel.toggle_detail_section(click.clone(), cx)
                    }))
                    .on_action(cx.listener(move |panel, _: &menu::Confirm, _, cx| {
                        panel.toggle_detail_section(keyboard.clone(), cx)
                    }))
                    .into_any_element()
                },
            ));
            if expanded {
                for (index, (name, detail)) in values.into_iter().enumerate() {
                    let id = format!("detail-value-{key:?}-{part}-{index}");
                    rows.push(SidebarRow::new(id.clone(), depth + 1, move |_, cx| {
                        let label = format!("{name} · {detail}");
                        tree_row(id.clone(), label.clone(), icon, None, cx)
                            .tooltip(crate::ui::components::Tooltip::text(label))
                            .into_any_element()
                    }));
                }
            }
        }
    }
}
