use crate::ui::components::prelude::*;
use gpui_kit::Entity;

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
        cx.set_global(UiLocale(language));
        cx.refresh_windows();
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

pub(super) fn apply_theme(preference: ThemePreference, cx: &mut App) {
    use gpui_kit::component::{Theme, ThemeMode};
    let mode = match preference {
        ThemePreference::Light => ThemeMode::Light,
        ThemePreference::Dark => ThemeMode::Dark,
        ThemePreference::System => {
            if matches!(
                cx.window_appearance(),
                gpui_kit::WindowAppearance::Dark | gpui_kit::WindowAppearance::VibrantDark
            ) {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            }
        }
    };
    Theme::change(mode, None, cx);
    cx.refresh_windows();
}
pub(super) fn refresh_active_theme(preference: ThemePreference, cx: &mut App) {
    apply_theme(preference, cx);
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

pub(super) struct UiLocale(pub UiLanguage);
impl gpui_kit::Global for UiLocale {}

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
