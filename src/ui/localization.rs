use crate::platform::{ThemePreference, UiLanguage};

pub(super) const fn text(
    language: UiLanguage,
    chinese: &'static str,
    english: &'static str,
) -> &'static str {
    match language {
        UiLanguage::Chinese => chinese,
        UiLanguage::English => english,
    }
}

pub(super) const fn theme_label(language: UiLanguage, theme: ThemePreference) -> &'static str {
    match theme {
        ThemePreference::System => text(language, "跟随系统", "System"),
        ThemePreference::Light => text(language, "浅色", "Light"),
        ThemePreference::Dark => text(language, "深色", "Dark"),
    }
}
