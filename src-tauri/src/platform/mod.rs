mod app_lifecycle;
mod events;
mod preferences;
pub(crate) mod sidecar;

pub(crate) use app_lifecycle::install_last_window_quit_policy;
pub use events::{UiEvent, UiEventBus, UiEventSinkHandle};
pub(crate) use preferences::{
    DesktopPreferences, NativePreferencesStore, ThemePreference, UiLanguage,
};
pub use sidecar::{
    SidecarControlHandle, SidecarEvent, SidecarHostHandle, SidecarRequest, SpawnedSidecar,
};
