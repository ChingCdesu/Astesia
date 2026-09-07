pub use gpui_kit::component::menu::{
    PopupMenu as ContextMenu, PopupMenuItem as ContextMenuEntry, PopupMenuItem as ContextMenuItem,
};
use gpui_kit::*;
pub enum IconPosition {
    Start,
}
pub trait MenuEntryExt: Sized {
    fn handler(self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self;
    fn toggleable(self, position: IconPosition, selected: bool) -> Self;
}
impl MenuEntryExt for ContextMenuEntry {
    fn handler(self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_click(move |_, window, cx| handler(window, cx))
    }
    fn toggleable(self, _: IconPosition, selected: bool) -> Self {
        self.checked(selected)
    }
}
pub trait MenuExt: Sized {
    fn header(self, label: impl Into<SharedString>) -> Self;
    fn custom_entry<E: IntoElement>(
        self,
        render: impl Fn(&mut Window, &mut App) -> E + 'static,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self;
}
impl MenuExt for ContextMenu {
    fn header(self, label: impl Into<SharedString>) -> Self {
        self.label(label)
    }
    fn custom_entry<E: IntoElement>(
        self,
        render: impl Fn(&mut Window, &mut App) -> E + 'static,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.item(ContextMenuEntry::element(render).handler(handler))
    }
}

pub fn menu_surface(menu: Entity<ContextMenu>) -> MenuSurface {
    let child = div()
        .font_family(if cfg!(target_os = "macos") {
            ".SystemUIFont"
        } else if cfg!(target_os = "windows") {
            "Segoe UI"
        } else {
            "sans-serif"
        })
        .font_weight(FontWeight::NORMAL)
        .child(menu)
        .into_any_element();
    MenuSurface { child }
}

// Kit menu rows set text_sm internally; scope its rem base to keep menu text at 12px.
const MENU_REM_SIZE: Pixels = px(12.0 / 0.875);

pub struct MenuSurface {
    child: AnyElement,
}

impl IntoElement for MenuSurface {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for MenuSurface {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (
            window.with_rem_size(Some(MENU_REM_SIZE), |window| {
                self.child.request_layout(window, cx)
            }),
            (),
        )
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_rem_size(Some(MENU_REM_SIZE), |window| {
            self.child.prepaint(window, cx);
        });
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_rem_size(Some(MENU_REM_SIZE), |window| self.child.paint(window, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use std::{cell::Cell, rc::Rc};

    struct MenuTest {
        menu: Entity<ContextMenu>,
    }
    impl Render for MenuTest {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            menu_surface(self.menu.clone())
        }
    }

    #[gpui_kit::test]
    fn menu_text_is_12px_without_changing_window_typography(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Dark, cx)
        });
        let measured = Rc::new(Cell::new(px(0.0)));
        let result = measured.clone();
        let (_, cx) = cx.add_window_view(|window, cx| MenuTest {
            menu: ContextMenu::build(window, cx, move |menu, _, _| {
                menu.item(ContextMenuEntry::element(move |_, _| {
                    let result = result.clone();
                    div().child("Copy").child(
                        canvas(
                            move |_, window, _| {
                                result
                                    .set(window.text_style().font_size.to_pixels(window.rem_size()))
                            },
                            |_, _, _, _| {},
                        )
                        .w(px(1.0))
                        .h(px(1.0)),
                    )
                }))
            }),
        });
        let original = cx.update(|window, _| window.rem_size());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(measured.get(), px(12.0));
        assert_eq!(cx.update(|window, _| window.rem_size()), original);
    }
}
