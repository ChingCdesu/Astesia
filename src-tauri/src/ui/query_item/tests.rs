use assets::Assets;
use gpui::TestAppContext;

use super::*;
use crate::{
    connection_repository::SharedConnectionRepository,
    credential_vault::test_support::MemoryCredentialVault,
    platform::DesktopPreferences,
    ui::{bind_editor_keys, sql_language},
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

#[gpui::test]
fn native_find_replace_preserves_focus_and_grouped_undo(cx: &mut TestAppContext) {
    cx.update(|cx| {
        Assets.load_test_fonts(cx);
        let settings = settings::SettingsStore::test(cx);
        cx.set_global(settings);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        release_channel::init(release_channel::AppVersion::load("0.0.0", None, None), cx);
        gpui_tokio::init(cx);
        editor::init(cx);
        sql_language::init(cx);
        bind_editor_keys(cx);
        bind_query_item_keys(cx);
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
    let search = item.read_with(cx, |item, _| item.search.clone());
    window
        .update(cx, |item, window, cx| item.focus(window, cx))
        .expect("query window");

    cx.run_until_parked();
    cx.simulate_keystrokes(window.into(), "cmd-f");
    cx.simulate_keystrokes(window.into(), "s e l e c t");
    cx.run_until_parked();
    assert!(!search.read_with(cx, |search, _| search.is_dismissed()));
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
    assert!(search.read_with(cx, |search, _| search.is_dismissed()));
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

#[gpui::test]
fn clearing_a_session_target_discards_its_chart(cx: &mut TestAppContext) {
    cx.update(|cx| {
        Assets.load_test_fonts(cx);
        let settings = settings::SettingsStore::test(cx);
        cx.set_global(settings);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        release_channel::init(release_channel::AppVersion::load("0.0.0", None, None), cx);
        gpui_tokio::init(cx);
        editor::init(cx);
        sql_language::init(cx);
        bind_editor_keys(cx);
        bind_query_item_keys(cx);
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
                    &[vec![Value::from("row"), Value::from(1)]],
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
