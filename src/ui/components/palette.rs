use gpui_kit::component::Theme;
use gpui_kit::*;

pub trait WorkspaceTheme {
    fn colors(&self) -> WorkspaceColors;
    fn status(&self) -> WorkspaceStatus;
}
#[derive(Clone, Copy)]
pub struct WorkspaceColors {
    pub background: Hsla,
    pub border: Hsla,
    pub border_focused: Hsla,
    pub editor_background: Hsla,
    pub element_background: Hsla,
    pub elevated_surface_background: Hsla,
    pub ghost_element_hover: Hsla,
    pub ghost_element_selected: Hsla,
    pub panel_background: Hsla,
    pub status_bar_background: Hsla,
    pub surface_background: Hsla,
    pub text: Hsla,
    pub text_accent: Hsla,
    pub text_muted: Hsla,
}
#[derive(Clone, Copy)]
pub struct WorkspaceStatus {
    pub error: Hsla,
    pub error_background: Hsla,
    pub error_border: Hsla,
    pub warning: Hsla,
    pub warning_background: Hsla,
    pub warning_border: Hsla,
    pub info: Hsla,
    pub info_background: Hsla,
    pub info_border: Hsla,
    pub success_background: Hsla,
    pub success_border: Hsla,
}
impl WorkspaceTheme for Theme {
    fn colors(&self) -> WorkspaceColors {
        WorkspaceColors {
            background: self.title_bar,
            border: self.border,
            border_focused: self.ring,
            editor_background: self.background,
            element_background: self.secondary,
            elevated_surface_background: self.popover,
            ghost_element_hover: self.list_hover,
            ghost_element_selected: self.list_active,
            panel_background: self.sidebar,
            status_bar_background: self.title_bar,
            surface_background: self.sidebar,
            text: self.foreground,
            text_accent: self.primary,
            text_muted: self.muted_foreground,
        }
    }
    fn status(&self) -> WorkspaceStatus {
        WorkspaceStatus {
            error: self.danger,
            error_background: self.danger.opacity(0.12),
            error_border: self.danger,
            warning: self.warning,
            warning_background: self.warning.opacity(0.12),
            warning_border: self.warning,
            info: self.info,
            info_background: self.info.opacity(0.12),
            info_border: self.info,
            success_background: self.success.opacity(0.12),
            success_border: self.success,
        }
    }
}
pub use gpui_kit::component::ActiveTheme;
#[derive(Clone, Copy, Default)]
pub enum Color {
    #[default]
    Default,
    Muted,
    Error,
    Warning,
    Success,
    Custom(Hsla),
    Accent,
    Info,
    Placeholder,
}
impl Color {
    pub fn color(self, cx: &App) -> Hsla {
        let t = cx.theme();
        match self {
            Self::Default => t.foreground,
            Self::Muted | Self::Placeholder => t.muted_foreground,
            Self::Error => t.danger,
            Self::Warning => t.warning,
            Self::Success => t.success,
            Self::Custom(c) => c,
            Self::Accent => t.primary,
            Self::Info => t.info,
        }
    }
}
#[derive(Clone, Copy)]
pub enum LabelSize {
    Small,
    XSmall,
    Custom(Rems),
}
impl LabelSize {
    pub fn rems(self) -> Rems {
        match self {
            Self::Small => rems(0.875),
            Self::XSmall => rems(0.75),
            Self::Custom(r) => r,
        }
    }
}
pub enum TextSize {
    Small,
    XSmall,
}
impl TextSize {
    pub fn rems(self, _: &App) -> Rems {
        match self {
            Self::Small => rems(0.875),
            Self::XSmall => rems(0.75),
        }
    }
}
pub enum DynamicSpacing {
    Base02,
    Base04,
    Base08,
    Base12,
    Base20,
    Base32,
    Base48,
}
impl DynamicSpacing {
    pub fn rems(self, _: &App) -> Rems {
        rems(match self {
            Self::Base02 => 0.125,
            Self::Base04 => 0.25,
            Self::Base08 => 0.5,
            Self::Base12 => 0.75,
            Self::Base20 => 1.25,
            Self::Base32 => 2.,
            Self::Base48 => 3.,
        })
    }
}
pub enum ElevationIndex {
    ModalSurface,
}
impl ElevationIndex {
    pub fn shadow(self, _: &App) -> Vec<BoxShadow> {
        vec![BoxShadow {
            inset: false,
            color: black().opacity(0.15),
            offset: point(px(0.), px(4.)),
            blur_radius: px(16.),
            spread_radius: px(0.),
        }]
    }
}
