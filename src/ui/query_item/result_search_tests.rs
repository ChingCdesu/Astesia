use super::*;

fn result(values: Vec<Vec<Value>>) -> StatementResult {
    StatementResult {
        sql: "SELECT 1".into(),
        success: true,
        error: None,
        columns: (0..2)
            .map(|i| crate::db::ColumnInfo {
                name: format!("c{i}"),
                data_type: "text".into(),
                nullable: true,
                is_primary_key: false,
                default_value: None,
                comment: None,
            })
            .collect(),
        rows: values,
        affected_rows: 0,
        execution_time_ms: 1,
    }
}

#[test]
fn literal_matches_preserve_utf8_offsets_row_order_and_display_values() {
    let result = result(vec![
        vec![Value::from("你好Mira mira"), Value::Null],
        vec![Value::from("samira"), serde_json::json!({"mira": true})],
    ]);
    let matches = find_matches(&result, "mira", false);
    assert_eq!(matches.len(), 4);
    assert_eq!(
        (matches[0].row, matches[0].column, matches[0].range.clone()),
        (0, 0, 6..10)
    );
    assert_eq!((matches[2].row, matches[2].column), (1, 0));
    assert_eq!(find_matches(&result, "mira", true).len(), 3);
    assert_eq!(find_matches(&result, "你好", true)[0].range, 0..6);
    assert_eq!(find_matches(&result, "null", false)[0].column, 1);
    assert!(find_matches(&result, "", false).is_empty());
    assert!(find_matches(&result, ".*", false).is_empty());
}

#[gpui_kit::test]
fn result_find_routes_focus_wraps_and_recomputes_after_result_switch(cx: &mut TestAppContext) {
    use crate::{
        connection_repository::SharedConnectionRepository,
        credential_vault::test_support::MemoryCredentialVault, platform::DesktopPreferences,
        ui::sql_language,
    };
    cx.update(|cx| {
        crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Light, cx)
    });
    let directory = tempfile::tempdir().unwrap();
    let repository = SharedConnectionRepository::new(
        directory.path().join("connections.sqlite3"),
        MemoryCredentialVault::shared(),
    );
    let application = Arc::new(Application::with_repository(repository));
    let settings = cx.new(|_| ShellSettings::new(DesktopPreferences::default(), None));
    let window = cx.add_window(|window, cx| {
        let editor = cx.new(|cx| sql_language::editor("SELECT 'editor';", window, cx));
        QueryItem::new(application, editor, settings, window, cx)
    });
    let item = window.root(cx).unwrap();
    window
        .update(cx, |item, window, cx| {
            item.state.set_target(Some(QueryTarget {
                connection_id: "local".into(),
                connection_name: "Local".into(),
                database: ":memory:".into(),
                db_type: DbType::SQLite,
                session_generation: 1,
            }));
            let request = item
                .state
                .begin_execution(
                    QueryDocument::new("SELECT 1; SELECT 2".into(), 0..0),
                    QueryExecutionScope::All,
                )
                .unwrap();
            item.state.finish_execution(
                &request,
                Ok(vec![
                    result(vec![vec![Value::from("mira mira"), Value::from("mira")]]),
                    result(vec![vec![Value::from("other"), Value::Null]]),
                ]),
            );
            window.focus(&item.result_focus, cx);
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes(window.into(), "cmd-f m i r a");
    cx.run_until_parked();
    item.read_with(cx, |item, cx| {
        assert!(item.result_search.open);
        assert_eq!(item.result_search.matches.len(), 3);
        assert_eq!(item.editor.read(cx).text(cx), "SELECT 'editor';");
        assert!(!item
            .editor
            .read(cx)
            .search_bar()
            .unwrap()
            .read(cx)
            .is_open());
    });
    cx.simulate_keystrokes(window.into(), "shift-enter");
    cx.run_until_parked();
    assert_eq!(item.read_with(cx, |item, _| item.result_search.current), 2);
    cx.simulate_keystrokes(window.into(), "enter");
    cx.run_until_parked();
    assert_eq!(item.read_with(cx, |item, _| item.result_search.current), 0);
    item.update(cx, |item, cx| {
        item.state.select_result(1);
        item.refresh_result_search(cx);
        cx.notify();
    });
    cx.run_until_parked();
    assert!(item.read_with(cx, |item, _| item.result_search.matches.is_empty()));
    cx.simulate_keystrokes(window.into(), "escape");
    window
        .update(cx, |item, window, cx| {
            assert!(!item.result_search.open);
            assert!(item.result_focus.is_focused(window));
            assert_eq!(item.result_search.input.read(cx).value().as_ref(), "mira");
        })
        .unwrap();
}
