use super::*;
use crate::db::{DbType, TableRef};

fn function(schema: &str, name: &str) -> FunctionInfo {
    FunctionInfo {
        schema: Some(schema.into()),
        name: name.into(),
        language: None,
        return_type: None,
        definition: None,
    }
}

#[test]
fn groups_functions_by_metadata_and_retains_overloads_and_function_only_schemas() {
    let mut catalog = DatabaseCatalogSnapshot::loading(DbType::PostgreSQL);
    catalog.apply(CatalogEntry::Schemas(CatalogSection::Ready(vec![
        "public".into(),
        "empty".into(),
    ])));
    catalog.apply(CatalogEntry::Tables(CatalogSection::Ready(vec![
        TableInfo {
            reference: TableRef::qualified("public", "orders"),
            row_count: None,
            comment: None,
        },
    ])));
    catalog.apply(CatalogEntry::Functions(CatalogSection::Ready(vec![
        function("public", "public.lookup(integer)"),
        function("public", "public.lookup(text)"),
        function("a.b", "a.b.lookup(public.custom_type)"),
    ])));
    let groups = schema_objects(&catalog);
    let public = &groups[&Some("public".into())];
    assert_eq!(public.tables.len(), 1);
    assert_eq!(public.functions.len(), 2);
    assert_eq!(public.functions[1].name, "public.lookup(text)");
    assert_eq!(groups[&Some("a.b".into())].functions.len(), 1);
    assert!(groups[&Some("a.b".into())].tables.is_empty());
    assert!(groups.contains_key(&Some("empty".into())));
}

#[test]
fn functions_remain_discoverable_when_tables_fail_or_are_still_loading() {
    let mut catalog = DatabaseCatalogSnapshot::loading(DbType::PostgreSQL);
    catalog.apply(CatalogEntry::Functions(CatalogSection::Ready(vec![
        function("audit", "audit.record()"),
    ])));
    assert_eq!(
        schema_objects(&catalog)[&Some("audit".into())]
            .functions
            .len(),
        1
    );
    catalog.apply(CatalogEntry::Tables(CatalogSection::Failed(
        "permission denied".into(),
    )));
    assert_eq!(
        schema_objects(&catalog)[&Some("audit".into())]
            .functions
            .len(),
        1
    );
}

#[gpui_kit::test]
fn schema_rows_keep_functions_under_their_parent_and_hide_collapsed_children(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (window, _directory) = crate::ui::connections::tests::sidebar_test_window(cx);
    let panel = window.root(cx).unwrap();
    let target = QueryTarget {
        connection_id: "primary".into(),
        connection_name: "primary".into(),
        database: "test".into(),
        db_type: DbType::PostgreSQL,
        session_generation: 7,
    };
    let mut catalog = DatabaseCatalogSnapshot::loading(DbType::PostgreSQL);
    catalog.apply(CatalogEntry::Schemas(CatalogSection::Ready(vec![
        "public".into(),
        "audit".into(),
    ])));
    catalog.apply(CatalogEntry::Tables(CatalogSection::Ready(vec![
        TableInfo {
            reference: TableRef::qualified("public", "orders"),
            row_count: None,
            comment: None,
        },
    ])));
    catalog.apply(CatalogEntry::Functions(CatalogSection::Ready(vec![
        function("public", "public.lookup(integer)"),
        function("audit", "audit.record()"),
    ])));
    panel.update(cx, |panel, cx| {
        let mut initial = vec![];
        panel.append_schema_catalog_rows(&target, &catalog, &mut initial);
        assert!(!initial.iter().any(|r| r.key.starts_with("function-")));
        assert_eq!(
            initial
                .iter()
                .filter(|r| r.key.starts_with("table-"))
                .count(),
            1
        );
        panel.catalog_folder_expansion.insert(
            CatalogFolderKey::new(&target, Some("public".into()), DatabaseObjectKind::Function),
            true,
        );
        panel.catalog_folder_expansion.insert(
            CatalogFolderKey::new(&target, Some("audit".into()), DatabaseObjectKind::Function),
            true,
        );
        let mut rows = vec![];
        panel.append_schema_catalog_rows(&target, &catalog, &mut rows);
        assert_eq!(
            rows.iter()
                .filter(|r| r.key.starts_with("folder-") && r.depth == 2)
                .count(),
            4
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.key.starts_with("function-") && r.depth == 3)
                .count(),
            2
        );
        for row in &rows {
            let _ = (row.render)(panel, cx);
        }
        let table_folder =
            CatalogFolderKey::new(&target, Some("public".into()), DatabaseObjectKind::Table);
        panel.toggle_catalog_folder(table_folder, true, cx);
        rows.clear();
        panel.append_schema_catalog_rows(&target, &catalog, &mut rows);
        assert!(!rows.iter().any(|r| r.key.starts_with("table-")));
        assert_eq!(
            rows.iter()
                .filter(|r| r.key.starts_with("function-"))
                .count(),
            2
        );
        assert!(panel
            .selected_sidebar_row
            .as_ref()
            .unwrap()
            .starts_with("folder-"));
        panel
            .collapsed_schemas
            .insert(("primary".into(), 7, "test".into(), "public".into()));
        rows.clear();
        panel.append_schema_catalog_rows(&target, &catalog, &mut rows);
        let functions = rows
            .iter()
            .filter(|r| r.key.starts_with("function-"))
            .collect::<Vec<_>>();
        assert_eq!(functions.len(), 1);
        assert!(functions[0].key.ends_with("audit.record()"));
    });
}

#[test]
fn older_function_payloads_do_not_guess_a_schema_from_the_name() {
    let function: FunctionInfo = serde_json::from_value(serde_json::json!({"name":"lookup(custom.type)","language":null,"return_type":null,"definition":null})).unwrap();
    assert_eq!(function.schema, None);
}
