mod assets;
mod button;
mod chart_view;
mod command_palette;
mod components;
mod connection_profile_form;
mod connections;
mod copy_table_form;
mod data_grid_item;
mod document_item;
mod editor_search;
mod engine_presentation;
mod er_diagram_item;
mod input_field;
mod localization;
mod mcp_service_item;
mod modal;
mod object_definition_item;
mod object_mutation_form;
mod performance_item;
mod query_item;
mod redis_item;
mod runtime;
mod shell;
mod sql_completion;
mod sql_language;
mod table_structure_item;
mod tabs;
mod task_center_item;
mod text_editor;
mod theme;
mod workspace;

use std::sync::Arc;

use gpui_kit::http_client::BlockedHttpClient;
use gpui_kit::{
    point, px, size, App, AppContext as _, Bounds, KeyBinding, QuitMode, TitlebarOptions,
    WindowBounds, WindowDecorations, WindowOptions,
};

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
    let (preferences, preferences_store, preferences_warning) = load_preferences();

    gpui_kit::application()
        .with_assets(self::assets::UiAssets)
        .with_http_client(Arc::new(BlockedHttpClient::new()))
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx| {
            cx.set_global(shell::UiLocale(preferences.language));
            install_last_window_quit_policy();
            initialize_editor_runtime(preferences.theme, cx);
            open_main_window(cx, preferences, preferences_store, preferences_warning);
            cx.activate(true);
        });
}

fn initialize_editor_runtime(theme: crate::platform::ThemePreference, cx: &mut App) {
    runtime::init(cx);
    gpui_kit::init(cx);
    theme::install(cx);
    apply_theme(theme, cx);
    cx.bind_keys([KeyBinding::new(
        "escape",
        modal::DismissModal,
        Some("AstesiaModal"),
    )]);
    bind_connection_profile_form_keys(cx);
    bind_connection_profiles_keys(cx);
    bind_data_grid_item_keys(cx);
    bind_query_item_keys(cx);
    text_editor::bind_keys(cx);
    editor_search::bind_keys(cx);
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
            appears_transparent: true,
            traffic_light_position: Some(point(px(9.0), px(9.0))),
        }),
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some(APP_IDENTIFIER.to_owned()),
        window_min_size: Some(size(px(960.0), px(600.0))),
        ..Default::default()
    };

    cx.open_window(window_options, move |window, cx| {
        let view = cx.new(|cx| {
            AstesiaRoot::new(
                preferences,
                preferences_store,
                preferences_warning,
                window,
                cx,
            )
        });
        cx.new(|cx| gpui_kit::component::Root::new(view, window, cx))
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
