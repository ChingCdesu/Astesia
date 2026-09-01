mod command_palette;
mod connection_profile_form;
mod connections;
mod engine_presentation;
mod localization;
mod query_item;
mod shell;
mod tabs;
mod workspace;

use std::sync::Arc;

use assets::Assets;
use editor::{
    actions::{
        Backspace, Backtab, Copy, Cut, Delete, LineDown, LineUp, MoveLeft, MoveRight, Newline,
        Paste, Redo, SelectAll, SelectDown, SelectLeft, SelectRight, SelectUp, Tab, Undo,
    },
    Editor,
};
use gpui::{
    px, size, App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBounds,
    WindowOptions,
};
use http_client::BlockedHttpClient;

use self::connection_profile_form::bind_connection_profile_form_keys;
use self::connections::bind_connection_profiles_keys;
use self::query_item::bind_query_item_keys;
use self::shell::apply_theme;
use self::workspace::{bind_workspace_keys, AstesiaRoot};
use crate::platform::{DesktopPreferences, NativePreferencesStore};

const APP_IDENTIFIER: &str = "com.astesia.app";
const INITIAL_QUERY: &str = "SELECT 1;\n";
pub fn run() {
    configure_zed_data_dir();
    let (preferences, preferences_store, preferences_warning) = load_preferences();

    gpui_platform::application()
        .with_assets(Assets)
        .with_http_client(Arc::new(BlockedHttpClient::new()))
        .run(|cx| {
            initialize_editor_runtime(preferences.theme, cx);
            cx.on_window_closed(|cx, _window_id| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
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
    bind_editor_keys(cx);
    bind_connection_profile_form_keys(cx);
    bind_connection_profiles_keys(cx);
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
        let editor = cx.new(|cx| {
            let mut editor = Editor::multi_line(window, cx);
            editor.set_text(INITIAL_QUERY, window, cx);
            editor
        });
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
    ]);
}
