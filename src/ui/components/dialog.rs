use gpui_kit::component::{self as kit, ActiveTheme as _};
use gpui_kit::{prelude::*, *};
#[derive(IntoElement)]
pub struct Modal {
    id: ElementId,
    scroll: ScrollHandle,
    header: Option<AnyElement>,
    footer: Option<AnyElement>,
    children: Vec<AnyElement>,
}
impl Modal {
    pub fn new(id: impl Into<ElementId>, scroll: Option<ScrollHandle>) -> Self {
        Self {
            id: id.into(),
            scroll: scroll.unwrap_or_default(),
            header: None,
            footer: None,
            children: vec![],
        }
    }
    pub fn header(mut self, el: impl IntoElement) -> Self {
        self.header = Some(el.into_any_element());
        self
    }
    pub fn footer(mut self, el: impl IntoElement) -> Self {
        self.footer = Some(el.into_any_element());
        self
    }
    pub fn section(self, el: impl IntoElement) -> Self {
        self.child(el)
    }
}
impl ParentElement for Modal {
    fn extend(&mut self, els: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(els);
    }
}
impl RenderOnce for Modal {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        kit::dialog::DialogContent::new()
            .size_full()
            .bg(cx.theme().popover)
            .children(self.header)
            .child(
                kit::v_flex()
                    .id(self.id)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(self.children),
            )
            .children(self.footer)
    }
}
#[derive(IntoElement)]
pub struct ModalHeader {
    title: SharedString,
    description: Option<SharedString>,
    dismiss: bool,
}
impl ModalHeader {
    pub fn new() -> Self {
        Self {
            title: "".into(),
            description: None,
            dismiss: true,
        }
    }
    pub fn headline(mut self, title: impl Into<SharedString>) -> Self {
        self.title = title.into();
        self
    }
    pub fn description(mut self, text: impl Into<SharedString>) -> Self {
        self.description = Some(text.into());
        self
    }
    pub fn show_dismiss_button(mut self, show: bool) -> Self {
        self.dismiss = show;
        self
    }
}
impl RenderOnce for ModalHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let language = cx
            .try_global::<crate::ui::shell::UiLocale>()
            .map_or(crate::platform::UiLanguage::Chinese, |locale| locale.0);
        kit::dialog::DialogHeader::new()
            .p_3()
            .child(
                kit::h_flex()
                    .justify_between()
                    .child(kit::dialog::DialogTitle::new().child(self.title))
                    .when(self.dismiss, |el| {
                        el.child(
                            kit::button::Button::new("dismiss-modal")
                                .icon(kit::IconName::Close)
                                .accessibility_label(crate::ui::localization::text(
                                    language,
                                    "关闭对话框",
                                    "Close dialog",
                                ))
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(
                                        Box::new(crate::ui::modal::DismissModal),
                                        cx,
                                    )
                                }),
                        )
                    }),
            )
            .when_some(self.description, |el, text| {
                el.child(kit::dialog::DialogDescription::new().child(text))
            })
    }
}
#[derive(IntoElement)]
pub struct ModalFooter {
    start: Option<AnyElement>,
    end: Option<AnyElement>,
}
impl ModalFooter {
    pub fn new() -> Self {
        Self {
            start: None,
            end: None,
        }
    }
    pub fn start_slot(mut self, e: impl IntoElement) -> Self {
        self.start = Some(e.into_any_element());
        self
    }
    pub fn end_slot(mut self, e: impl IntoElement) -> Self {
        self.end = Some(e.into_any_element());
        self
    }
}
impl RenderOnce for ModalFooter {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        kit::dialog::DialogFooter::new().p_3().child(
            kit::h_flex()
                .w_full()
                .justify_between()
                .children(self.start)
                .children(self.end),
        )
    }
}
