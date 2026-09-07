use gpui_kit::TestAppContext;

use super::*;
use crate::{
    connection_repository::SharedConnectionRepository,
    credential_vault::test_support::MemoryCredentialVault, platform::DesktopPreferences,
    ui::sql_language,
};

#[test]
fn result_cells_preserve_scalar_and_structured_values() {
    assert_eq!(display_value(&Value::Null), "NULL");
    assert_eq!(display_value(&Value::String("二".to_string())), "二");
    assert_eq!(
        display_value(&serde_json::json!({ "ok": true })),
        "{\"ok\":true}"
    );
}

#[gpui_kit::test]
fn native_find_replace_preserves_focus_and_grouped_undo(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Light, cx);
    });

    let directory = tempfile::tempdir().expect("query search repository directory");
    let repository = SharedConnectionRepository::new(
        directory.path().join("connections.sqlite3"),
        MemoryCredentialVault::shared(),
    );
    let application = Arc::new(Application::with_repository(repository));
    let settings = cx.new(|_| ShellSettings::new(DesktopPreferences::default(), None));
    let mut editor = None;
    let window = cx.add_window(|window, cx| {
        let query_editor = cx.new(|cx| sql_language::editor("SELECT 1;\nSELECT 1;", window, cx));
        editor = Some(query_editor.clone());
        QueryItem::new(application, query_editor, settings, window, cx)
    });
    let item = window.root(cx).expect("query item root");
    let editor = editor.expect("query editor");
    let search = editor.read_with(cx, |editor, _| editor.search_bar().unwrap().clone());
    window
        .update(cx, |item, window, cx| item.focus(window, cx))
        .expect("query window");

    cx.run_until_parked();
    cx.simulate_keystrokes(window.into(), "cmd-f");
    cx.simulate_keystrokes(window.into(), "s e l e c t");
    cx.run_until_parked();
    assert!(!search.read_with(cx, |search, _| !search.is_open()));
    assert_eq!(
        search.read_with(cx, |search, cx| search.query(cx)),
        "select"
    );
    cx.simulate_keystrokes(window.into(), "cmd-enter");
    assert!(
        item.read_with(cx, |item, _| item.state.error().is_none()),
        "query execution shortcuts must not escape the SQL editor"
    );

    cx.simulate_keystrokes(window.into(), "cmd-shift-h");
    cx.simulate_keystrokes(window.into(), "u p d a t e");
    assert_eq!(
        search.update(cx, |search, cx| search.replacement(cx)),
        "update"
    );
    cx.simulate_keystrokes(window.into(), "cmd-enter");
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |editor, cx| editor.text(cx)),
        "update 1;\nupdate 1;"
    );

    cx.simulate_keystrokes(window.into(), "escape cmd-z");
    assert!(search.read_with(cx, |search, _| !search.is_open()));
    assert_eq!(
        editor.read_with(cx, |editor, cx| editor.text(cx)),
        "SELECT 1;\nSELECT 1;"
    );

    cx.simulate_keystrokes(window.into(), "cmd-enter");
    assert_eq!(
        item.read_with(cx, |item, _| item.state.error().map(|error| error.code)),
        Some("query_target_required")
    );
}

#[gpui_kit::test]
fn clearing_a_session_target_discards_its_chart(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Light, cx);
    });

    let directory = tempfile::tempdir().expect("query chart repository directory");
    let repository = SharedConnectionRepository::new(
        directory.path().join("connections.sqlite3"),
        MemoryCredentialVault::shared(),
    );
    let application = Arc::new(Application::with_repository(repository));
    let settings = cx.new(|_| ShellSettings::new(DesktopPreferences::default(), None));
    let chart_settings = settings.clone();
    let window = cx.add_window(|window, cx| {
        let query_editor = cx.new(|cx| sql_language::editor("SELECT 1", window, cx));
        QueryItem::new(application, query_editor, settings, window, cx)
    });
    let item = window.root(cx).expect("query item root");

    item.update(cx, |item, cx| {
        item.state.set_target(Some(QueryTarget {
            connection_id: "sqlite".to_string(),
            connection_name: "Local".to_string(),
            database: ":memory:".to_string(),
            db_type: DbType::SQLite,
            session_generation: 1,
        }));
        item.chart = Some(cx.new(|cx| {
            ChartView::new(
                ChartModel::from_names(
                    vec!["label".to_string(), "value".to_string()],
                    vec![vec![Value::from("row"), Value::from(1)]],
                ),
                chart_settings,
                cx,
            )
        }));
        item.showing_chart = true;
        item.clear_target(cx);
    });

    item.read_with(cx, |item, _| {
        assert!(item.state.target().is_none());
        assert!(item.chart.is_none());
        assert!(!item.showing_chart);
    });
}

#[gpui_kit::test]
fn hidden_charts_do_not_retain_query_results(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Light, cx);
    });

    let directory = tempfile::tempdir().expect("lazy chart repository directory");
    let application = Arc::new(Application::with_repository(
        SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        ),
    ));
    let settings = cx.new(|_| ShellSettings::new(DesktopPreferences::default(), None));
    let window = cx.add_window(|window, cx| {
        let editor = cx.new(|cx| sql_language::editor("SELECT 1", window, cx));
        QueryItem::new(application, editor, settings, window, cx)
    });
    let item = window.root(cx).expect("lazy chart query item");
    let retained_result = item.update(cx, |item, cx| {
        item.state.set_target(Some(QueryTarget {
            connection_id: "sqlite".to_string(),
            connection_name: "Local".to_string(),
            database: ":memory:".to_string(),
            db_type: DbType::SQLite,
            session_generation: 1,
        }));
        let request = item
            .state
            .begin_execution(
                QueryDocument::new("SELECT 1".to_string(), 0..0),
                QueryExecutionScope::All,
            )
            .unwrap();
        let result = StatementResult::from_query_result(
            "SELECT 1".to_string(),
            crate::db::QueryResult {
                rows: vec![vec![Value::from("x".repeat(1_024)), Value::from(1)]],
                ..Default::default()
            },
        );
        assert!(item.state.finish_execution(&request, Ok(vec![result])));
        item.sync_chart(cx);
        assert!(item.chart.is_none());
        Arc::downgrade(&item.state.shared_active_result().unwrap())
    });
    assert_eq!(retained_result.strong_count(), 1);

    window
        .update(cx, |item, window, cx| {
            item.toggle_chart(&ClickEvent::default(), window, cx);
            assert!(item.showing_chart);
            assert!(item.chart.is_some());
        })
        .unwrap();
    assert_eq!(retained_result.strong_count(), 2);

    window
        .update(cx, |item, window, cx| {
            item.toggle_chart(&ClickEvent::default(), window, cx);
            assert!(!item.showing_chart);
        })
        .unwrap();
    assert_eq!(retained_result.strong_count(), 1);

    item.update(cx, |item, cx| {
        item.state.clear_results();
        item.sync_chart(cx);
        assert!(item.chart.is_none());
    });
    assert!(retained_result.upgrade().is_none());
}

#[gpui_kit::test]
#[ignore = "Release memory workload; run alone with --ignored --nocapture"]
fn release_memory_query_chart_workload(cx: &mut TestAppContext) {
    use std::{io::Write as _, time::Duration};

    use crate::db::{ColumnInfo, QueryResult};

    let row_count = std::env::var("ASTESIA_MEMORY_ROW_COUNT")
        .map(|value| value.parse::<usize>().expect("memory workload row count"))
        .unwrap_or(100_000);
    assert!(row_count > 0, "memory workload must contain rows");

    cx.update(|cx| {
        crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Light, cx);
    });

    let directory = tempfile::tempdir().expect("memory workload repository directory");
    let repository = SharedConnectionRepository::new(
        directory.path().join("connections.sqlite3"),
        MemoryCredentialVault::shared(),
    );
    let application = Arc::new(Application::with_repository(repository));
    let settings = cx.new(|_| ShellSettings::new(DesktopPreferences::default(), None));
    let sql = "SELECT id, value, payload FROM memory_workload";
    let window = cx.add_window(|window, cx| {
        let query_editor = cx.new(|cx| sql_language::editor(sql, window, cx));
        QueryItem::new(application, query_editor, settings, window, cx)
    });
    let item = window.root(cx).expect("memory workload query item");
    item.update(cx, |item, cx| {
        item.state.set_target(Some(QueryTarget {
            connection_id: "memory-workload".to_string(),
            connection_name: "Memory workload".to_string(),
            database: ":memory:".to_string(),
            db_type: DbType::SQLite,
            session_generation: 1,
        }));
        let request = item
            .state
            .begin_execution(
                QueryDocument::new(sql.to_string(), 0..0),
                QueryExecutionScope::All,
            )
            .expect("memory workload execution request");
        let columns = [("id", "integer"), ("value", "integer"), ("payload", "text")]
            .into_iter()
            .map(|(name, data_type)| ColumnInfo {
                name: name.to_string(),
                data_type: data_type.to_string(),
                nullable: false,
                is_primary_key: name == "id",
                default_value: None,
                comment: None,
            })
            .collect();
        let rows = (0..row_count)
            .map(|row| {
                vec![
                    Value::from(row),
                    Value::from(row % 1_000),
                    Value::from("x".repeat(1_024)),
                ]
            })
            .collect();
        let result = StatementResult::from_query_result(
            sql.to_string(),
            QueryResult {
                columns,
                rows,
                affected_rows: 0,
                execution_time_ms: 1,
            },
        );
        assert!(item.state.finish_execution(&request, Ok(vec![result])));
        item.sync_chart(cx);
        cx.notify();
    });
    cx.run_until_parked();
    assert!(!item.read_with(cx, |item, _| item.showing_chart));
    println!(
        "MEMORY_STAGE query_hidden pid={} rows={row_count} payload_bytes=1024",
        std::process::id()
    );
    std::io::stdout()
        .flush()
        .expect("flush memory stage marker");
    std::thread::sleep(Duration::from_secs(5));

    window
        .update(cx, |item, window, cx| {
            item.toggle_chart(&ClickEvent::default(), window, cx);
        })
        .expect("memory workload chart window");
    cx.run_until_parked();
    assert!(item.read_with(cx, |item, _| item.showing_chart && item.chart.is_some()));
    println!(
        "MEMORY_STAGE query_visible pid={} rows={row_count} payload_bytes=1024",
        std::process::id()
    );
    std::io::stdout()
        .flush()
        .expect("flush memory stage marker");
    std::thread::sleep(Duration::from_secs(5));
}
