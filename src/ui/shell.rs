use gpui::Entity;
use serde_json::json;
use zed_ui::prelude::*;

use crate::platform::{DesktopPreferences, NativePreferencesStore, ThemePreference, UiLanguage};

const MAX_VISIBLE_NOTIFICATIONS: usize = 4;

pub(super) struct ShellSettings {
    preferences: DesktopPreferences,
    store: Option<NativePreferencesStore>,
}

impl ShellSettings {
    pub(super) fn new(
        preferences: DesktopPreferences,
        store: Option<NativePreferencesStore>,
    ) -> Self {
        Self { preferences, store }
    }

    pub(super) fn theme(&self) -> ThemePreference {
        self.preferences.theme
    }

    pub(super) fn language(&self) -> UiLanguage {
        self.preferences.language
    }

    pub(super) fn sidebar_visible(&self) -> bool {
        self.preferences.sidebar_visible
    }

    pub(super) fn set_theme(
        &mut self,
        theme: ThemePreference,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.preferences.theme = theme;
        apply_theme(theme, cx);
        self.persist_and_notify(cx)
    }

    pub(super) fn set_language(
        &mut self,
        language: UiLanguage,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.preferences.language = language;
        self.persist_and_notify(cx)
    }

    pub(super) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        self.preferences.sidebar_visible = !self.preferences.sidebar_visible;
        self.persist_and_notify(cx)
    }

    fn persist_and_notify(&self, cx: &mut Context<Self>) -> Result<(), String> {
        let result = self.persist();
        cx.notify();
        result
    }

    fn persist(&self) -> Result<(), String> {
        self.store
            .as_ref()
            .ok_or_else(|| "原生偏好设置不可用，本次更改不会在重启后保留".to_string())?
            .save(&self.preferences)
    }
}

pub(super) fn apply_theme(theme: ThemePreference, cx: &mut App) {
    let mode = match theme {
        ThemePreference::Light => "light",
        ThemePreference::Dark => "dark",
        ThemePreference::System => "system",
    };
    let settings = json!({
        "telemetry": {
            "diagnostics": false,
            "metrics": false
        },
        "disable_ai": true,
        "auto_update": false,
        "theme": {
            "mode": mode,
            "light": "One Light",
            "dark": "One Dark"
        }
    });
    settings::SettingsStore::update(cx, |store, cx| {
        store
            .set_user_settings(&settings.to_string(), cx)
            .result()
            .expect("failed to apply isolated editor settings");
    });
    refresh_active_theme(theme, cx);
}

pub(super) fn refresh_active_theme(theme_preference: ThemePreference, cx: &mut App) {
    let appearance = match theme_preference {
        ThemePreference::Light => theme::Appearance::Light,
        ThemePreference::Dark => theme::Appearance::Dark,
        ThemePreference::System => theme::SystemAppearance::global(cx).0,
    };
    let theme_name = match appearance {
        theme::Appearance::Light => "One Light",
        theme::Appearance::Dark => "One Dark",
    };
    let registry = theme::ThemeRegistry::default_global(cx);
    let active_theme = registry.get(theme_name).unwrap_or_else(|_| {
        registry
            .list()
            .into_iter()
            .find(|candidate| candidate.appearance == appearance)
            .and_then(|candidate| registry.get(&candidate.name).ok())
            .unwrap_or_else(|| cx.theme().clone())
    });
    theme::GlobalTheme::update_theme(cx, active_theme);
    cx.refresh_windows();
    debug_assert!(match theme_preference {
        ThemePreference::Light => cx.theme().appearance() == theme::Appearance::Light,
        ThemePreference::Dark => cx.theme().appearance() == theme::Appearance::Dark,
        ThemePreference::System => true,
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NotificationTone {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Notification {
    id: u64,
    tone: NotificationTone,
    message: String,
}

pub(super) struct NotificationCenter {
    next_id: u64,
    notifications: Vec<Notification>,
}

impl NotificationCenter {
    pub(super) fn new() -> Self {
        Self {
            next_id: 0,
            notifications: Vec::new(),
        }
    }

    pub(super) fn push(
        &mut self,
        tone: NotificationTone,
        message: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.next_id = self.next_id.wrapping_add(1);
        self.notifications.push(Notification {
            id: self.next_id,
            tone,
            message: message.into(),
        });
        if self.notifications.len() > MAX_VISIBLE_NOTIFICATIONS {
            self.notifications.remove(0);
        }
        cx.notify();
    }

    fn dismiss(&mut self, id: u64, cx: &mut Context<Self>) {
        self.notifications
            .retain(|notification| notification.id != id);
        cx.notify();
    }
}

impl Render for NotificationCenter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = cx.theme().status();
        v_flex()
            .absolute()
            .top_3()
            .right_3()
            .w(px(360.0))
            .gap_2()
            .children(self.notifications.iter().map(|notification| {
                let id = notification.id;
                let (foreground, background, border) = match notification.tone {
                    NotificationTone::Info => {
                        (status.info, status.info_background, status.info_border)
                    }
                    NotificationTone::Warning => (
                        status.warning,
                        status.warning_background,
                        status.warning_border,
                    ),
                    NotificationTone::Error => {
                        (status.error, status.error_background, status.error_border)
                    }
                };
                h_flex()
                    .id(format!("notification-{id}"))
                    .items_start()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(border)
                    .bg(background)
                    .shadow_md()
                    .child(
                        Label::new(notification.message.clone())
                            .size(LabelSize::XSmall)
                            .color(Color::Custom(foreground))
                            .line_clamp(4)
                            .flex_1(),
                    )
                    .child(
                        IconButton::new(format!("dismiss-notification-{id}"), IconName::Close)
                            .icon_size(IconSize::XSmall)
                            .on_click(cx.listener(move |center, _, _, cx| {
                                center.dismiss(id, cx);
                            })),
                    )
            }))
    }
}

pub(super) fn notify_preference_error(
    center: &Entity<NotificationCenter>,
    error: String,
    cx: &mut App,
) {
    center.update(cx, |center, cx| {
        center.push(NotificationTone::Error, error, cx);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_and_language_cycles_include_every_supported_value() {
        assert_eq!(ThemePreference::System.next(), ThemePreference::Light);
        assert_eq!(ThemePreference::Light.next(), ThemePreference::Dark);
        assert_eq!(ThemePreference::Dark.next(), ThemePreference::System);
        assert_eq!(UiLanguage::Chinese.next(), UiLanguage::English);
        assert_eq!(UiLanguage::English.next(), UiLanguage::Chinese);
    }
}
