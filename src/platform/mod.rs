mod app_lifecycle;
mod data_directory;
mod events;
mod preferences;
mod process_sidecar;
pub(crate) mod sidecar;

pub(crate) use app_lifecycle::install_last_window_quit_policy;
pub(crate) use data_directory::application_data_directory;
pub use events::{UiEvent, UiEventBus, UiEventSinkHandle};
pub(crate) use preferences::{
    DesktopPreferences, NativePreferencesStore, ThemePreference, UiLanguage,
};
pub(crate) use process_sidecar::ProcessSidecarHost;
pub use sidecar::{
    SidecarControlHandle, SidecarEvent, SidecarHostHandle, SidecarRequest, SpawnedSidecar,
};
