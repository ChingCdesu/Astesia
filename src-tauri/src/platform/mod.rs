mod events;
mod preferences;
pub(crate) mod sidecar;

pub use events::{UiEvent, UiEventBus, UiEventSinkHandle};
pub(crate) use preferences::{
    DesktopPreferences, NativePreferencesStore, ThemePreference, UiLanguage,
};
pub use sidecar::{
    SidecarControlHandle, SidecarEvent, SidecarHostHandle, SidecarRequest, SpawnedSidecar,
};
