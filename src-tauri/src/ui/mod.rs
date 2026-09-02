mod command_palette;
mod connection_profile_form;
mod connections;
mod data_grid_item;
mod engine_presentation;
mod localization;
mod object_definition_item;
mod object_mutation_form;
mod query_item;
mod shell;
mod sql_completion;
mod sql_language;
mod table_structure_item;
mod tabs;
mod workspace;

use std::sync::Arc;

use assets::Assets;
use editor::actions::{
    Backspace, Backtab, Cancel, ComposeCompletion, ConfirmCompletion, ContextMenuFirst,
    ContextMenuLast, ContextMenuNext, ContextMenuPrevious, Copy, Cut, Delete, LineDown, LineUp,
    MoveLeft, MoveRight, Newline, Paste, Redo, SelectAll, SelectDown, SelectLeft, SelectRight,
    SelectUp, ShowCompletions, Tab, Undo,
};
use gpui::{
    px, size, App, AppContext as _, Bounds, KeyBinding, QuitMode, TitlebarOptions, WindowBounds,
    WindowOptions,
};
use http_client::BlockedHttpClient;

use self::connection_profile_form::bind_connection_profile_form_keys;
use self::connections::bind_connection_profiles_keys;
use self::data_grid_item::bind_data_grid_item_keys;
use self::query_item::bind_query_item_keys;
use self::shell::apply_theme;
use self::workspace::{bind_workspace_keys, AstesiaRoot};
use crate::platform::{
    install_last_window_quit_policy, DesktopPreferences, NativePreferencesStore,
};

const APP_IDENTIFIER: &str = "com.astesia.app";
const INITIAL_QUERY: &str = "SELECT 1;\n";
pub fn run() {
    configure_zed_data_dir();
    let (preferences, preferences_store, preferences_warning) = load_preferences();

    gpui_platform::application()
        .with_assets(Assets)
        .with_http_client(Arc::new(BlockedHttpClient::new()))
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx| {
            install_last_window_quit_policy();
            initialize_editor_runtime(preferences.theme, cx);
            open_main_window(cx, preferences, preferences_store, preferences_warning);
            cx.activate(true);
        });
}

fn initialize_editor_runtime(theme: crate::platform::ThemePreference, cx: &mut App) {
    gpui_tokio::init(cx);

    let app_version = release_channel::AppVersion::load(env!("CARGO_PKG_VERSION"), None, None);
    release_channel::init(app_version, cx);
    Assets
        .load_fonts(cx)
        .expect("failed to load embedded Zed fonts");

    settings::init(cx);
    theme_settings::init(theme::LoadThemes::All(Box::new(Assets)), cx);
    apply_theme(theme, cx);
    editor::init(cx);
    sql_language::init(cx);
    bind_editor_keys(cx);
    bind_connection_profile_form_keys(cx);
    bind_connection_profiles_keys(cx);
    bind_data_grid_item_keys(cx);
    bind_query_item_keys(cx);
    bind_workspace_keys(cx);
}

fn open_main_window(
    cx: &mut App,
    preferences: DesktopPreferences,
    preferences_store: Option<NativePreferencesStore>,
    preferences_warning: Option<String>,
) {
    let bounds = Bounds::centered(None, size(px(1280.0), px(800.0)), cx);
    let window_options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("Astesia - Database Manager".into()),
            ..Default::default()
        }),
        app_id: Some(APP_IDENTIFIER.to_owned()),
        window_min_size: Some(size(px(960.0), px(600.0))),
        ..Default::default()
    };

    cx.open_window(window_options, move |window, cx| {
        theme_settings::setup_ui_font(window, cx);
        let editor = cx.new(|cx| sql_language::editor(INITIAL_QUERY, window, cx));
        cx.new(|cx| {
            AstesiaRoot::new(
                editor,
                preferences,
                preferences_store,
                preferences_warning,
                window,
                cx,
            )
        })
    })
    .expect("failed to open the Astesia window");
}

fn load_preferences() -> (
    DesktopPreferences,
    Option<NativePreferencesStore>,
    Option<String>,
) {
    let store = match NativePreferencesStore::new_default() {
        Ok(store) => store,
        Err(error) => return (DesktopPreferences::default(), None, Some(error)),
    };
    match store.load() {
        Ok(preferences) => (preferences, Some(store), None),
        Err(error) => (DesktopPreferences::default(), Some(store), Some(error)),
    }
}

fn configure_zed_data_dir() {
    let zed_data_dir = dirs::data_dir()
        .expect("failed to resolve the application data directory")
        .join(APP_IDENTIFIER)
        .join("zed-runtime");

    // Zed caches its path roots on first access and otherwise targets the user's Zed profile.
    paths::set_custom_data_dir(&zed_data_dir.to_string_lossy());
}

fn bind_editor_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-space", ShowCompletions, Some("Editor")),
        KeyBinding::new("backspace", Backspace, Some("Editor")),
        KeyBinding::new("delete", Delete, Some("Editor")),
        KeyBinding::new("enter", Newline, Some("Editor")),
        KeyBinding::new("tab", Tab, Some("Editor")),
        KeyBinding::new("shift-tab", Backtab, Some("Editor")),
        KeyBinding::new("left", MoveLeft, Some("Editor")),
        KeyBinding::new("right", MoveRight, Some("Editor")),
        KeyBinding::new("up", LineUp, Some("Editor")),
        KeyBinding::new("down", LineDown, Some("Editor")),
        KeyBinding::new("shift-left", SelectLeft, Some("Editor")),
        KeyBinding::new("shift-right", SelectRight, Some("Editor")),
        KeyBinding::new("shift-up", SelectUp, Some("Editor")),
        KeyBinding::new("shift-down", SelectDown, Some("Editor")),
        KeyBinding::new("cmd-a", SelectAll, Some("Editor")),
        KeyBinding::new("cmd-c", Copy, Some("Editor")),
        KeyBinding::new("cmd-x", Cut, Some("Editor")),
        KeyBinding::new("cmd-v", Paste, Some("Editor")),
        KeyBinding::new("cmd-z", Undo, Some("Editor")),
        KeyBinding::new("cmd-shift-z", Redo, Some("Editor")),
        KeyBinding::new("ctrl-a", SelectAll, Some("Editor")),
        KeyBinding::new("ctrl-c", Copy, Some("Editor")),
        KeyBinding::new("ctrl-x", Cut, Some("Editor")),
        KeyBinding::new("ctrl-v", Paste, Some("Editor")),
        KeyBinding::new("ctrl-z", Undo, Some("Editor")),
        KeyBinding::new("ctrl-shift-z", Redo, Some("Editor")),
        KeyBinding::new(
            "enter",
            ConfirmCompletion::default(),
            Some("Editor && showing_completions"),
        ),
        KeyBinding::new(
            "tab",
            ComposeCompletion::default(),
            Some("Editor && showing_completions"),
        ),
        KeyBinding::new("escape", Cancel, Some("Editor && showing_completions")),
        KeyBinding::new(
            "up",
            ContextMenuPrevious,
            Some("Editor && showing_completions"),
        ),
        KeyBinding::new(
            "down",
            ContextMenuNext,
            Some("Editor && showing_completions"),
        ),
        KeyBinding::new(
            "pageup",
            ContextMenuFirst,
            Some("Editor && showing_completions"),
        ),
        KeyBinding::new(
            "pagedown",
            ContextMenuLast,
            Some("Editor && showing_completions"),
        ),
    ]);
}

#[cfg(test)]
mod tests {
    use gpui::{EntityInputHandler as _, TestAppContext};

    use super::*;

    #[gpui::test]
    fn standalone_editor_groups_ime_composition_for_undo(cx: &mut TestAppContext) {
        cx.update(|cx| {
            Assets.load_test_fonts(cx);
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(release_channel::AppVersion::load("0.0.0", None, None), cx);
            editor::init(cx);
            sql_language::init(cx);
        });

        let editor = cx.add_window(|window, cx| sql_language::editor("", window, cx));

        editor
            .update(cx, |editor, window, cx| {
                editor.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
                editor.replace_and_mark_text_in_range(None, "nihao", Some(5..5), window, cx);
                assert_eq!(editor.marked_text_range(window, cx), Some(0..5));

                editor.replace_text_in_range(None, "你好", window, cx);
                assert_eq!(editor.text(cx), "你好");
                assert_eq!(editor.marked_text_range(window, cx), None);

                editor.undo(&Default::default(), window, cx);
                assert_eq!(editor.text(cx), "");
                editor.redo(&Default::default(), window, cx);
                assert_eq!(editor.text(cx), "你好");
            })
            .expect("editor window");
    }
}
