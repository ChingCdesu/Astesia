use super::*;
use gpui_kit::component::{
    self as kit, button::ButtonVariants as _, ActiveTheme as _, Disableable as _, Selectable as _,
    Sizable as _,
};
use gpui_kit::{prelude::*, *};

#[derive(IntoElement)]
pub struct Label {
    inner: kit::label::Label,
    color: Color,
}
impl Label {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            inner: kit::label::Label::new(text),
            color: Color::Default,
        }
    }
    pub fn size(mut self, size: LabelSize) -> Self {
        self.inner = self.inner.text_size(size.rems());
        self
    }
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
    pub fn buffer_font(mut self, cx: &App) -> Self {
        self.inner = self.inner.font_family(cx.theme().mono_font_family.clone());
        self
    }
}
impl Styled for Label {
    fn style(&mut self) -> &mut StyleRefinement {
        self.inner.style()
    }
}
impl RenderOnce for Label {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.inner.text_color(self.color.color(cx))
    }
}

#[derive(Clone, Copy)]
pub enum IconName {
    ArrowDown,
    ArrowUp,
    Check,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    Close,
    Command,
    Copy,
    Download,
    Eraser,
    Filter,
    FolderOpen,
    ListTodo,
    ListTree,
    LoaderCircle,
    Pencil,
    Play,
    PlayFilled,
    Plus,
    RotateCw,
    Search,
    Server,
    Settings,
    Trash,
    TriangleAlert,
}
impl kit::IconNamed for IconName {
    fn path(self) -> SharedString {
        match self {
            Self::ArrowDown => kit::IconName::ArrowDown.path(),
            Self::ArrowUp => kit::IconName::ArrowUp.path(),
            Self::Check => kit::IconName::Check.path(),
            Self::ChevronDown => kit::IconName::ChevronDown.path(),
            Self::ChevronLeft => kit::IconName::ChevronLeft.path(),
            Self::ChevronRight => kit::IconName::ChevronRight.path(),
            Self::Close => kit::IconName::Close.path(),
            Self::Command => "icons/astesia/command.svg".into(),
            Self::Copy => kit::IconName::Copy.path(),
            Self::Download => "icons/astesia/download.svg".into(),
            Self::Eraser => "icons/astesia/eraser.svg".into(),
            Self::Filter => "icons/astesia/filter.svg".into(),
            Self::FolderOpen => kit::IconName::FolderOpen.path(),
            Self::ListTodo => "icons/astesia/list_todo.svg".into(),
            Self::ListTree => "icons/astesia/list_tree.svg".into(),
            Self::LoaderCircle => kit::IconName::LoaderCircle.path(),
            Self::Pencil => "icons/astesia/pencil.svg".into(),
            Self::Play => kit::IconName::Play.path(),
            Self::PlayFilled => "icons/astesia/play-filled.svg".into(),
            Self::Plus => kit::IconName::Plus.path(),
            Self::RotateCw => kit::IconName::RotateCw.path(),
            Self::Search => kit::IconName::Search.path(),
            Self::Server => "icons/astesia/server.svg".into(),
            Self::Settings => kit::IconName::Settings.path(),
            Self::Trash => "icons/astesia/trash.svg".into(),
            Self::TriangleAlert => kit::IconName::TriangleAlert.path(),
        }
    }
}
#[derive(Clone, Copy)]
pub enum IconSize {
    Small,
    XSmall,
}
impl IconSize {
    fn pixels(self) -> Pixels {
        px(match self {
            Self::Small => 16.,
            Self::XSmall => 12.,
        })
    }
}
#[derive(IntoElement)]
pub struct Icon {
    inner: kit::Icon,
    color: Color,
}
impl Icon {
    pub fn new(name: IconName) -> Self {
        Self {
            inner: kit::Icon::new(name),
            color: Color::Default,
        }
    }
    pub fn from_path(path: impl Into<SharedString>) -> Self {
        Self {
            inner: kit::Icon::default().path(path),
            color: Color::Default,
        }
    }
    pub fn size(mut self, size: IconSize) -> Self {
        self.inner = self.inner.size(size.pixels());
        self
    }
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
}
impl Styled for Icon {
    fn style(&mut self) -> &mut StyleRefinement {
        self.inner.style()
    }
}
impl RenderOnce for Icon {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.inner.text_color(self.color.color(cx))
    }
}

pub enum ButtonSize {
    Compact,
}
pub enum TintColor {
    Error,
}
pub enum ButtonStyle {
    Filled,
    Transparent,
    Outlined,
    Tinted(TintColor),
}
type TooltipBuilder = dyn Fn(&mut Window, &mut App) -> AnyView;
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    inner: kit::button::Button,
    label: SharedString,
    tooltip: Option<std::rc::Rc<TooltipBuilder>>,
}
impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        let id = id.into();
        let label: SharedString = label.into();
        Self {
            id: id.clone(),
            inner: kit::button::Button::new(id)
                .when(!label.is_empty(), |button| button.label(label.clone()))
                .ghost()
                .small(),
            label,
            tooltip: None,
        }
    }
    pub fn size(mut self, _: ButtonSize) -> Self {
        self.inner = self.inner.compact();
        self
    }
    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.inner = match style {
            ButtonStyle::Filled => self.inner.primary(),
            ButtonStyle::Transparent => self.inner.ghost(),
            ButtonStyle::Outlined => self.inner.outline(),
            ButtonStyle::Tinted(TintColor::Error) => self.inner.danger(),
        };
        self
    }
    pub fn toggle_state(mut self, selected: bool) -> Self {
        self.inner = self.inner.selected(selected);
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.inner = self.inner.disabled(disabled);
        self
    }
    pub fn on_click(
        mut self,
        callback: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.inner = self.inner.on_click(callback);
        self
    }
    pub fn tab_index(mut self, index: isize) -> Self {
        self.inner = self.inner.tab_index(index);
        self
    }
    pub fn key_binding(mut self, binding: Option<KeyBinding>) -> Self {
        if let Some(binding) = binding {
            let label = self.label.clone();
            self.tooltip = Some(std::rc::Rc::new(move |window, cx| {
                kit::tooltip::Tooltip::new(label.clone())
                    .action(binding.action.as_ref(), None)
                    .build(window, cx)
            }));
        }
        self
    }
    pub fn tooltip(mut self, build: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.tooltip = Some(std::rc::Rc::new(build));
        self
    }
}
impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        self.inner.style()
    }
}
impl ParentElement for Button {
    fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
        self.inner.extend(children);
    }
}
impl InteractiveElement for Button {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.inner.interactivity()
    }
}
impl RenderOnce for Button {
    fn render(mut self, _: &mut Window, _: &mut App) -> impl IntoElement {
        if let Some(build) = self.tooltip {
            self.inner
                .interactivity()
                .tooltip(move |window, cx| build(window, cx));
        }
        self.inner
    }
}

pub type ButtonLike = Button;
#[derive(IntoElement)]
pub struct IconButton {
    inner: Button,
    icon: IconName,
}
impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: IconName) -> Self {
        let mut inner = Button::new(id, "");
        inner.inner = inner.inner.icon(icon);
        Self { inner, icon }
    }
    pub fn icon_size(mut self, size: IconSize) -> Self {
        self.inner.inner = self
            .inner
            .inner
            .icon(kit::Icon::new(self.icon).size(size.pixels()))
            .compact();
        self
    }
    pub fn on_click(
        mut self,
        callback: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.inner = self.inner.on_click(callback);
        self
    }
    pub fn tooltip(mut self, build: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.inner = self.inner.tooltip(build);
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.inner = self.inner.disabled(disabled);
        self
    }
}
impl Styled for IconButton {
    fn style(&mut self) -> &mut StyleRefinement {
        Styled::style(&mut self.inner)
    }
}
impl InteractiveElement for IconButton {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.inner.interactivity()
    }
}
impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.inner
    }
}

#[derive(IntoElement)]
pub struct Indicator {
    color: Color,
}
impl Indicator {
    pub fn dot() -> Self {
        Self {
            color: Color::Default,
        }
    }
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
}
impl RenderOnce for Indicator {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div().size_1p5().rounded_full().bg(self.color.color(cx))
    }
}
pub struct Tooltip;
impl Tooltip {
    pub fn text(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
        let text = text.into();
        move |window, cx| kit::tooltip::Tooltip::new(text.clone()).build(window, cx)
    }
    pub fn for_action(text: impl Into<SharedString>, action: &dyn Action, cx: &mut App) -> AnyView {
        cx.new(|_| TooltipView {
            text: text.into(),
            action: action.boxed_clone(),
        })
        .into()
    }
}
struct TooltipView {
    text: SharedString,
    action: Box<dyn Action>,
}
impl Render for TooltipView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        kit::tooltip::Tooltip::new(self.text.clone())
            .action(self.action.as_ref(), None)
            .build(window, cx)
    }
}
pub struct KeyBinding {
    action: Box<dyn Action>,
}
impl KeyBinding {
    pub fn for_action(action: &dyn Action, _: &App) -> Option<Self> {
        Some(Self {
            action: action.boxed_clone(),
        })
    }
}

impl Button {
    pub fn end_icon(mut self, icon: impl IntoElement) -> Self {
        self.inner = self.inner.child(icon);
        self
    }
    pub fn popup_menu(
        self,
        build: impl Fn(ContextMenu, &mut Window, &mut Context<ContextMenu>) -> ContextMenu + 'static,
    ) -> impl IntoElement {
        super::MenuTrigger::new(self, build)
    }
}
impl IconButton {
    pub fn size(mut self, size: ButtonSize) -> Self {
        self.inner = self.inner.size(size);
        self
    }
    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.inner = self.inner.style(style);
        self
    }
    pub fn popup_menu(
        self,
        build: impl Fn(ContextMenu, &mut Window, &mut Context<ContextMenu>) -> ContextMenu + 'static,
    ) -> impl IntoElement {
        self.inner.popup_menu(build)
    }
}

impl StatefulInteractiveElement for Button {}
impl StatefulInteractiveElement for IconButton {}
impl Button {
    pub fn loading(mut self, loading: bool) -> Self {
        self.inner = self.inner.loading(loading);
        self
    }
    pub fn selected_style(mut self, style: ButtonStyle) -> Self {
        if self.inner.is_selected() {
            self = self.style(style);
        }
        self
    }
    pub fn start_icon(mut self, icon: impl IntoElement) -> Self {
        self.inner = self.inner.child(icon);
        self
    }
}
impl Label {
    pub fn weight(mut self, weight: FontWeight) -> Self {
        self.inner = self.inner.font_weight(weight);
        self
    }
}

impl Button {
    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.inner = self.inner.accessibility_label(label);
        self
    }
}
impl IconButton {
    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.inner = self.inner.aria_label(label);
        self
    }
}

impl Button {
    pub(super) fn element_id(&self) -> &ElementId {
        &self.id
    }
}
