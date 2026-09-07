use gpui_kit::component::{Theme, ThemeConfig, ThemeConfigColors, ThemeMode};
use gpui_kit::App;
use std::rc::Rc;

pub(super) fn install(cx: &mut App) {
    cx.text_system()
        .add_fonts(vec![
            include_bytes!("fonts/geist-mono/GeistMono-Regular.ttf")
                .as_slice()
                .into(),
            include_bytes!("fonts/geist-mono/GeistMono-Medium.ttf")
                .as_slice()
                .into(),
            include_bytes!("fonts/geist-mono/GeistMono-SemiBold.ttf")
                .as_slice()
                .into(),
            include_bytes!("fonts/geist-mono/GeistMono-Bold.ttf")
                .as_slice()
                .into(),
        ])
        .expect("bundled Geist Mono fonts must load");
    let theme = Theme::global_mut(cx);
    theme.light_theme = Rc::new(config(ThemeMode::Light));
    theme.dark_theme = Rc::new(config(ThemeMode::Dark));
}

pub(super) fn restore_selection_surfaces(cx: &mut App) {
    let theme = Theme::global_mut(cx);
    // Kit caps configured selection alpha at 0.2; Figma specifies opaque row fills.
    let selected = theme.sidebar_accent;
    theme.list_active = selected;
    theme.table_active = selected;
    theme.tokens.list_active = selected.into();
    theme.tokens.table_active = selected.into();
    Theme::sync_base(cx);
}

fn config(mode: ThemeMode) -> ThemeConfig {
    let color = |light, dark| if mode.is_dark() { dark } else { light };
    let workspace = color("#dcdcdd", "#3b414d");
    let surface = color("#ebebec", "#2f343e");
    let editor = color("#fafafa", "#282c33");
    let element = color("#ebebec", "#2e343e");
    let border = color("#c9c9ca", "#464b57");
    let text = color("#242529", "#dce0e5");
    let muted = color("#58585a", "#a9afbc");
    let hover = color("#dfdfe0", "#363c46");
    let selected = color("#cacaca", "#454a56");
    let success = color("#669f59", "#a1c181");
    let warning = color("#a48819", "#dec184");
    let error = color("#d36151", "#d07277");
    let mut colors = ThemeConfigColors::default();
    colors.background = Some(editor.into());
    colors.table = Some(editor.into());
    colors.table_even = Some(editor.into());
    colors.tab_active = Some(editor.into());
    colors.title_bar = Some(workspace.into());
    colors.sidebar = Some(surface.into());
    colors.popover = Some(surface.into());
    colors.tab = Some(surface.into());
    colors.tab_bar = Some(surface.into());
    colors.list = Some(surface.into());
    colors.list_even = Some(surface.into());
    colors.list_head = Some(surface.into());
    colors.table_head = Some(surface.into());
    colors.table_foot = Some(surface.into());
    colors.accordion = Some(surface.into());
    colors.group_box = Some(surface.into());
    colors.secondary = Some(element.into());
    colors.button = Some(element.into());
    colors.border = Some(border.into());
    colors.input = Some(border.into());
    colors.sidebar_border = Some(border.into());
    colors.title_bar_border = Some(border.into());
    colors.table_row_border = Some(border.into());
    colors.list_active_border = Some(border.into());
    colors.table_active_border = Some(border.into());
    colors.foreground = Some(text.into());
    colors.sidebar_foreground = Some(text.into());
    colors.popover_foreground = Some(text.into());
    colors.tab_active_foreground = Some(text.into());
    colors.table_head_foreground = Some(text.into());
    colors.table_foot_foreground = Some(text.into());
    colors.button_foreground = Some(text.into());
    colors.secondary_foreground = Some(text.into());
    colors.accent_foreground = Some(text.into());
    colors.sidebar_accent_foreground = Some(text.into());
    colors.group_box_foreground = Some(text.into());
    colors.muted_foreground = Some(muted.into());
    colors.tab_foreground = Some(muted.into());
    colors.list_hover = Some(hover.into());
    colors.table_hover = Some(hover.into());
    colors.button_hover = Some(hover.into());
    colors.secondary_hover = Some(hover.into());
    colors.accent = Some(hover.into());
    colors.list_active = Some(selected.into());
    colors.table_active = Some(selected.into());
    colors.sidebar_accent = Some(selected.into());
    colors.button_active = Some(selected.into());
    colors.secondary_active = Some(selected.into());
    colors.muted = Some(selected.into());
    colors.selection = Some(selected.into());
    colors.success = Some(success.into());
    colors.warning = Some(warning.into());
    colors.danger = Some(error.into());
    let mut highlight = if mode.is_dark() {
        gpui_kit::component::highlighter::HighlightTheme::default_dark()
            .style
            .clone()
    } else {
        gpui_kit::component::highlighter::HighlightTheme::default_light()
            .style
            .clone()
    };
    let parse = |value: &str| {
        gpui_kit::Hsla::from(gpui_kit::Rgba::try_from(value).expect("valid theme color"))
    };
    highlight.editor_background = Some(parse(editor));
    highlight.editor_foreground = Some(parse(text));
    highlight.editor_gutter_background = Some(parse(editor));
    highlight.editor_line_number = Some(parse(muted));
    highlight.editor_active_line_number = Some(parse(text));
    highlight.editor_active_line = Some(parse(editor));
    colors.button_primary = Some(element.into());
    colors.button_primary_foreground = Some(text.into());
    ThemeConfig {
        name: if mode.is_dark() {
            "Astesia Dark"
        } else {
            "Astesia Light"
        }
        .into(),
        mode,
        font_family: Some("Geist Mono".into()),
        mono_font_family: Some("Geist Mono".into()),
        mono_font_size: Some(14.0),
        highlight: Some(highlight),
        radius: Some(5),
        colors,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::ThemePreference;
    use crate::ui::components::WorkspaceTheme;
    use gpui_kit::{rgb, TestAppContext};

    #[gpui_kit::test]
    fn appearance_changes_keep_figma_surfaces_and_component_tokens(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::ui::initialize_editor_runtime(ThemePreference::Light, cx);
            for (mode, editor, surface, workspace, selected) in [
                (ThemeMode::Dark, 0x282c33, 0x2f343e, 0x3b414d, 0x454a56),
                (ThemeMode::Light, 0xfafafa, 0xebebec, 0xdcdcdd, 0xcacaca),
                (ThemeMode::Dark, 0x282c33, 0x2f343e, 0x3b414d, 0x454a56),
            ] {
                crate::ui::shell::apply_theme(
                    if mode.is_dark() {
                        ThemePreference::Dark
                    } else {
                        ThemePreference::Light
                    },
                    cx,
                );
                let theme = Theme::global(cx);
                let colors = theme.colors();
                assert_eq!(colors.editor_background, rgb(editor).into());
                assert_eq!(colors.surface_background, rgb(surface).into());
                assert_eq!(colors.status_bar_background, rgb(workspace).into());
                assert_eq!(colors.background, rgb(workspace).into());
                assert_eq!(theme.highlight_theme.appearance, mode);
                assert_eq!(
                    theme.highlight_theme.style.editor_background,
                    Some(colors.editor_background)
                );
                assert_eq!(
                    theme.highlight_theme.style.editor_gutter_background,
                    Some(colors.editor_background)
                );
                assert_eq!(
                    theme.highlight_theme.style.editor_foreground,
                    Some(theme.foreground)
                );
                assert_eq!(theme.tab_active, colors.editor_background);
                assert_eq!(theme.popover, colors.surface_background);
                assert_eq!(theme.list_active, rgb(selected).into());
                assert_eq!(theme.tokens.tab_active.color, theme.tab_active);
                assert_eq!(theme.tokens.list_active.color, theme.list_active);
            }
        });
    }
}
