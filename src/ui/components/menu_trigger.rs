use super::{Button, ContextMenu};
use gpui_kit::{prelude::*, *};
use std::rc::Rc;

type MenuBuilder = dyn Fn(ContextMenu, &mut Window, &mut Context<ContextMenu>) -> ContextMenu;
#[derive(IntoElement)]
pub struct MenuTrigger {
    button: Button,
    build: Rc<MenuBuilder>,
}
impl MenuTrigger {
    pub fn new(
        button: Button,
        build: impl Fn(ContextMenu, &mut Window, &mut Context<ContextMenu>) -> ContextMenu + 'static,
    ) -> Self {
        Self {
            button,
            build: Rc::new(build),
        }
    }
}
#[derive(Default)]
struct MenuState {
    bounds: Bounds<Pixels>,
    menu: Option<Entity<ContextMenu>>,
    subscription: Option<Subscription>,
}
impl RenderOnce for MenuTrigger {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.button.element_id().clone(), cx, |_, _| {
            MenuState::default()
        });
        let menu = state.read(cx).menu.clone();
        let position = state.read(cx).bounds.bottom_left();
        let paint_state = state.clone();
        let build = self.build;
        div()
            .relative()
            .child(self.button.on_click(move |_, window, cx| {
                state.update(cx, |state, cx| {
                    if state.menu.take().is_some() {
                        state.subscription = None;
                        cx.notify();
                        return;
                    }
                    let previous = window.focused(cx);
                    let build = build.clone();
                    let menu =
                        ContextMenu::build(window, cx, move |menu, w, cx| build(menu, w, cx));
                    state.subscription = Some(cx.subscribe_in(
                        &menu,
                        window,
                        move |state, menu, _: &DismissEvent, window, cx| {
                            if menu.focus_handle(cx).contains_focused(window, cx) {
                                if let Some(focus) = previous.as_ref() {
                                    window.focus(focus, cx);
                                }
                            }
                            state.menu = None;
                            cx.notify();
                        },
                    ));
                    window.focus(&menu.focus_handle(cx), cx);
                    state.menu = Some(menu);
                    cx.notify();
                });
            }))
            .child(
                canvas(
                    |bounds, _, _| bounds,
                    move |_, bounds, _, cx| {
                        paint_state.update(cx, |state, _| state.bounds = bounds)
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(menu.map(|menu| {
                deferred(
                    anchored()
                        .position(position)
                        .anchor(Anchor::TopLeft)
                        .child(super::menu_surface(menu)),
                )
                .with_priority(3)
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use std::cell::Cell;
    struct Harness(Rc<Cell<bool>>);
    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let opened = self.0.clone();
            div().tab_group().child(MenuTrigger::new(
                Button::new("menu-test", "Open menu"),
                move |menu, _, _| {
                    opened.set(true);
                    menu.label("Menu opened")
                },
            ))
        }
    }
    #[gpui_kit::test]
    fn keyboard_activation_opens_the_menu(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Light, cx)
        });
        let opened = Rc::new(Cell::new(false));
        let state = opened.clone();
        let (_, cx) = cx.add_window_view(|_, _| Harness(state));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.focus_next(cx);
            window.draw(cx).clear(cx);
            assert!(window.focused(cx).is_some());
        });
        let keystroke = Keystroke::parse("enter").unwrap();
        cx.simulate_event(KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(KeyUpEvent { keystroke });
        assert!(opened.get());
    }
}
