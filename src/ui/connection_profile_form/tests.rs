use super::*;
use crate::{
    connection_repository::SharedConnectionRepository,
    credential_vault::test_support::MemoryCredentialVault,
};
use gpui_kit::TestAppContext;

#[gpui_kit::test]
fn engine_changes_preserve_identity_and_invalidate_test_results(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Light, cx);
    });
    let directory = tempfile::tempdir().expect("form repository");
    let application = Arc::new(Application::with_repository(
        SharedConnectionRepository::new(
            directory.path().join("connections.sqlite3"),
            MemoryCredentialVault::shared(),
        ),
    ));
    let window = cx.add_window(|window, cx| {
        ConnectionProfileForm::new(
            application,
            ConnectionProfileFormMode::Create,
            UiLanguage::Chinese,
            window,
            cx,
        )
    });
    window
        .update(cx, |form, window, cx| {
            assert_eq!(form.db_type, DbType::MySQL);
            assert!(form.fields.name.read(cx).is_empty(cx));
            assert!(form.fields.group_name.read(cx).is_empty(cx));
            set_text(&form.fields.name, "本地开发", window, cx);
            set_text(&form.fields.group_name, "Development", window, cx);
            set_text(&form.fields.tags, "local, test", window, cx);
            form.test_notice = Some(FormNotice::success("Test passed", form.language));
            form.select_db_type(DbType::SQLite, window, cx);
            assert_eq!(field_text(&form.fields.name, cx), "本地开发");
            assert_eq!(field_text(&form.fields.group_name, cx), "Development");
            assert_eq!(field_text(&form.fields.tags, cx), "local, test");
            assert!(form.fields.username.read(cx).is_empty(cx));
            assert!(form.fields.database.read(cx).is_empty(cx));
            assert!(form.test_notice.is_none());
            form.select_db_type(DbType::PostgreSQL, window, cx);
            assert_eq!(field_text(&form.fields.port, cx), "5432");
            assert_eq!(field_text(&form.fields.username, cx), "postgres");
            assert_eq!(field_text(&form.fields.endpoint, cx), "localhost");
            form.operation = FormOperation::Testing;
            form.select_db_type(DbType::Redis, window, cx);
            assert_eq!(form.db_type, DbType::PostgreSQL);
            form.operation = FormOperation::Idle;
            form.test_notice = Some(FormNotice::success("Test passed", form.language));
            form.handle_input_event(&ErasedEditorEvent::Change, cx);
            assert!(form.test_notice.is_none());
        })
        .expect("form window");
}
