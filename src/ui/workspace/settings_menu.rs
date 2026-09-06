use super::*;

impl AstesiaWorkspace {
    pub(super) fn settings_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let language = self.settings.read(cx).language();
        let workspace = cx.entity().downgrade();
        IconButton::new("open-settings", IconName::Settings)
            .icon_size(IconSize::Small)
            .aria_label(text(language, "设置", "Settings"))
            .popup_menu(move |menu, _, cx| {
                let Some(owner) = workspace.upgrade() else {
                    return menu;
                };
                let settings = owner.read(cx).settings.read(cx);
                let language = settings.language();
                let theme = settings.theme();
                let language_owner = workspace.clone();
                let theme_owner = workspace.clone();
                menu.custom_entry(
                    move |_, _| {
                        settings_row(
                            text(language, "语言", "Language"),
                            match language {
                                UiLanguage::Chinese => "简体中文",
                                UiLanguage::English => "English",
                            },
                        )
                    },
                    move |_, cx| {
                        language_owner
                            .update(cx, |workspace, cx| {
                                workspace.set_language(language.next(), cx);
                            })
                            .ok();
                    },
                )
                .custom_entry(
                    move |_, _| {
                        settings_row(
                            text(language, "主题", "Theme"),
                            theme_label(language, theme),
                        )
                    },
                    move |_, cx| {
                        theme_owner
                            .update(cx, |workspace, cx| {
                                workspace.set_theme(theme.next(), cx);
                            })
                            .ok();
                    },
                )
            })
    }
}

fn settings_row(label: &'static str, value: &'static str) -> AnyElement {
    h_flex()
        .id("settings-row-label")
        .role(gpui_kit::Role::Label)
        .aria_value(format!("{label}: {value}"))
        .w(px(208.0))
        .justify_between()
        .child(Label::new(label).size(LabelSize::Small))
        .child(Label::new(value).size(LabelSize::Small).color(Color::Muted))
        .into_any_element()
}
