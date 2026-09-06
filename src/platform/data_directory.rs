use std::path::PathBuf;

pub(crate) fn application_data_directory() -> Option<PathBuf> {
    // Native acceptance runs need disposable profiles and preferences without changing
    // the user's application state. Release builds always use the platform directory.
    #[cfg(debug_assertions)]
    if let Some(directory) = std::env::var_os("ASTESIA_DEBUG_DATA_DIR") {
        let directory = PathBuf::from(directory);
        if directory.is_absolute() {
            return Some(directory);
        }
    }
    dirs::data_dir().map(|directory| directory.join("com.astesia.app"))
}
