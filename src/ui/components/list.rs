use gpui_kit::component::{self as kit};
use gpui_kit::{prelude::*, *};
#[derive(IntoElement)]
pub struct ListItem {
    start: Option<AnyElement>,
    end: Option<AnyElement>,
    children: Vec<AnyElement>,
    inner: kit::list::ListItem,
}
pub enum ListItemSpacing {
    Dense,
}
impl ListItem {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            start: None,
            end: None,
            children: vec![],
            inner: kit::list::ListItem::new(id),
        }
    }
    pub fn spacing(mut self, _: ListItemSpacing) -> Self {
        self.inner = self.inner.py_0p5();
        self
    }
    pub fn inset(mut self, inset: bool) -> Self {
        if !inset {
            self.inner = self.inner.rounded_none();
        }
        self
    }
    pub fn start_slot(mut self, element: impl IntoElement) -> Self {
        self.start = Some(element.into_any_element());
        self
    }
    pub fn end_slot(mut self, element: impl IntoElement) -> Self {
        self.end = Some(element.into_any_element());
        self
    }
    pub fn toggle_state(mut self, selected: bool) -> Self {
        self.inner = self.inner.selected(selected);
        self
    }
    pub fn on_click(
        mut self,
        callback: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.inner = self.inner.on_click(callback);
        self
    }
}
impl Styled for ListItem {
    fn style(&mut self) -> &mut StyleRefinement {
        self.inner.style()
    }
}
impl ParentElement for ListItem {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}
impl InteractiveElement for ListItem {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.inner.interactivity()
    }
}
impl RenderOnce for ListItem {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.inner.child(
            kit::h_flex()
                .w_full()
                .gap_1()
                .children(self.start)
                .child(kit::h_flex().flex_1().min_w_0().children(self.children))
                .children(self.end),
        )
    }
}

impl StatefulInteractiveElement for ListItem {}
impl ListItem {
    pub fn aria_role(self, role: Role) -> Self {
        self.role(role)
    }
}

impl ListItem {
    pub fn on_secondary_mouse_down(
        mut self,
        callback: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.inner = self.inner.on_mouse_down(MouseButton::Right, callback);
        self
    }
}
