use gpui_kit::component::{v_flex, ActiveTheme as _};
use gpui_kit::{prelude::*, *};
gpui_kit::actions!(astesia_modal, [DismissModal]);
pub(super) enum DismissDecision {
    Dismiss(bool),
}
pub(super) trait ModalView: Render + Focusable + EventEmitter<DismissEvent> + Sized {
    fn fade_out_background(&self) -> bool {
        true
    }
    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut Context<Self>) -> DismissDecision {
        DismissDecision::Dismiss(true)
    }
}
type DismissGuard = dyn Fn(&mut Window, &mut App) -> bool;
struct ActiveModal {
    view: AnyView,
    may_dismiss: Box<DismissGuard>,
    previous_focus: Option<FocusHandle>,
    dim: bool,
    _subscription: Subscription,
}
pub(super) struct ModalLayer {
    active: Option<ActiveModal>,
}
impl ModalLayer {
    pub(super) fn new() -> Self {
        Self { active: None }
    }
    pub(super) fn has_active_modal(&self) -> bool {
        self.active.is_some()
    }
    pub(super) fn active_modal<T: 'static>(&self) -> Option<Entity<T>> {
        self.active.as_ref()?.view.clone().downcast::<T>().ok()
    }
    pub(super) fn hide_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(active) = self.active.as_ref() else {
            return true;
        };
        if !(active.may_dismiss)(window, cx) {
            return false;
        }
        let active = self.active.take().unwrap();
        if let Some(focus) = active.previous_focus {
            window.focus(&focus, cx);
        }
        cx.notify();
        true
    }
    pub(super) fn toggle_modal<T: ModalView + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        build: impl FnOnce(&mut Window, &mut Context<T>) -> T,
    ) {
        let same = self.active_modal::<T>().is_some();
        if !self.hide_modal(window, cx) || same {
            return;
        }
        let previous_focus = window.focused(cx);
        let view = cx.new(|cx| build(window, cx));
        let guard = view.clone();
        let dim = view.read(cx).fade_out_background();
        let subscription =
            cx.subscribe_in(&view, window, |layer, _, _: &DismissEvent, window, cx| {
                layer.hide_modal(window, cx);
            });
        window.focus(&view.focus_handle(cx), cx);
        self.active = Some(ActiveModal {
            view: view.into(),
            dim,
            previous_focus,
            _subscription: subscription,
            may_dismiss: Box::new(move |window, cx| {
                guard.update(cx, |view, cx| {
                    matches!(
                        view.on_before_dismiss(window, cx),
                        DismissDecision::Dismiss(true)
                    )
                })
            }),
        });
        cx.notify();
    }
}
impl Render for ModalLayer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(active) = &self.active else {
            return div().into_any_element();
        };
        v_flex()
            .id("workspace-modal-layer")
            .absolute()
            .inset_0()
            .size_full()
            .occlude()
            .items_center()
            .justify_center()
            .when(active.dim, |el| el.bg(cx.theme().overlay))
            .key_context("AstesiaModal")
            .tab_group()
            .on_action(cx.listener(|layer, _: &DismissModal, window, cx| {
                layer.hide_modal(window, cx);
            }))
            .child(
                div()
                    .id("modal-backdrop")
                    .absolute()
                    .inset_0()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|layer, _, window, cx| {
                            layer.hide_modal(window, cx);
                        }),
                    ),
            )
            .child(
                div()
                    .id("active-modal")
                    .relative()
                    .role(Role::Dialog)
                    .child(active.view.clone()),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    struct Form {
        busy: bool,
        focus: FocusHandle,
    }
    impl Render for Form {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().track_focus(&self.focus).child("Test form")
        }
    }
    impl Focusable for Form {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.focus.clone()
        }
    }
    impl EventEmitter<DismissEvent> for Form {}
    impl ModalView for Form {
        fn on_before_dismiss(&mut self, _: &mut Window, _: &mut Context<Self>) -> DismissDecision {
            DismissDecision::Dismiss(!self.busy)
        }
    }
    #[gpui_kit::test]
    fn a_busy_modal_blocks_dismissal_and_replacement(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Light, cx)
        });
        let window = cx.add_window(|_, _| ModalLayer::new());
        window
            .update(cx, |layer, window, cx| {
                layer.toggle_modal(window, cx, |_, cx| Form {
                    busy: true,
                    focus: cx.focus_handle(),
                });
                let original = layer.active_modal::<Form>().unwrap();
                assert!(!layer.hide_modal(window, cx));
                layer.toggle_modal::<Form>(window, cx, |_, _| {
                    panic!("busy form must not be replaced")
                });
                assert_eq!(
                    layer.active_modal::<Form>().unwrap().entity_id(),
                    original.entity_id()
                );
                original.update(cx, |form, _| form.busy = false);
                assert!(layer.hide_modal(window, cx));
                assert!(!layer.has_active_modal());
            })
            .unwrap();
    }
}
