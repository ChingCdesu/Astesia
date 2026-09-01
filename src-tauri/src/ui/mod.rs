mod connection_profile_form;
mod connections;
mod engine_presentation;
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
use self::workspace::AstesiaRoot;

const APP_IDENTIFIER: &str = "com.astesia.app";
const INITIAL_QUERY: &str = "SELECT 1;\n";
const ISOLATED_ZED_SETTINGS: &str = r#"{
  "telemetry": {
    "diagnostics": false,
    "metrics": false
  },
  "disable_ai": true,
  "auto_update": false
}"#;

pub fn run() {
    configure_zed_data_dir();

    gpui_platform::application()
        .with_assets(Assets)
        .with_http_client(Arc::new(BlockedHttpClient::new()))
        .run(|cx| {
            initialize_editor_runtime(cx);
            cx.on_window_closed(|cx, _window_id| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            open_main_window(cx);
            cx.activate(true);
        });
}

fn initialize_editor_runtime(cx: &mut App) {
    gpui_tokio::init(cx);

    let app_version = release_channel::AppVersion::load(env!("CARGO_PKG_VERSION"), None, None);
    release_channel::init(app_version, cx);
    Assets
        .load_fonts(cx)
        .expect("failed to load embedded Zed fonts");

    settings::init(cx);
    settings::SettingsStore::update(cx, |store, cx| {
        store
            .set_user_settings(ISOLATED_ZED_SETTINGS, cx)
            .result()
            .expect("failed to apply isolated editor settings");
    });
    theme_settings::init(theme::LoadThemes::JustBase, cx);
    editor::init(cx);
    bind_editor_keys(cx);
    bind_connection_profile_form_keys(cx);
    bind_connection_profiles_keys(cx);
}

fn open_main_window(cx: &mut App) {
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
        cx.new(|cx| AstesiaRoot::new(editor, window, cx))
    })
    .expect("failed to open the Astesia window");
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
