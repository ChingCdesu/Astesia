use super::components::{Color, Label, LabelSize};
use gpui_kit::component::{
    input::{Input, InputEvent, InputState},
    ActiveTheme as _,
};
use gpui_kit::{prelude::*, *};
pub(super) struct InputField {
    state: Entity<InputState>,
    label: SharedString,
    tab_index: isize,
    code: bool,
    readonly: bool,
    error: Option<SharedString>,
    last_value: SharedString,
    _observation: Subscription,
    _subscription: Subscription,
}
impl InputField {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>, placeholder: &str) -> Self {
        let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder.to_owned()));
        let subscription = cx.subscribe(&state, |_, _, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                cx.emit(event.clone());
            }
            cx.notify();
        });
        let last_value = state.read(cx).value();
        let observation = cx.observe(&state, |this, state, cx| {
            let value = state.read(cx).value();
            if this.last_value != value {
                this.last_value = value;
                cx.emit(InputEvent::Change);
                cx.notify();
            }
        });
        Self {
            state,
            label: "".into(),
            tab_index: 0,
            code: false,
            readonly: false,
            error: None,
            last_value,
            _observation: observation,
            _subscription: subscription,
        }
    }
    pub(super) fn password(
        window: &mut Window,
        cx: &mut Context<Self>,
        placeholder: &str,
        tab_index: isize,
    ) -> Self {
        let field = Self::new(window, cx, placeholder).tab_index(tab_index);
        field
            .state
            .update(cx, |state, cx| state.set_masked(true, window, cx));
        field
    }
    pub(super) fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = label.into();
        self
    }
    pub(super) fn tab_index(mut self, index: isize) -> Self {
        self.tab_index = index;
        self
    }
    pub(super) fn code_value(mut self) -> Self {
        self.code = true;
        self
    }
    pub(super) fn text(&self, cx: &App) -> String {
        self.state.read(cx).value().to_string()
    }
    pub(super) fn is_empty(&self, cx: &App) -> bool {
        self.text(cx).trim().is_empty()
    }
    pub(super) fn set_text(
        &self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.state.update(cx, |s, cx| s.set_value(text, window, cx));
    }
    pub(super) fn clear(&self, window: &mut Window, cx: &mut App) {
        self.set_text("", window, cx);
    }
    pub(super) fn set_label(&mut self, label: &str, cx: &mut Context<Self>) {
        self.label = label.to_owned().into();
        cx.notify();
    }
    pub(super) fn set_error(
        &mut self,
        error: Option<impl Into<SharedString>>,
        cx: &mut Context<Self>,
    ) {
        self.error = error.map(Into::into);
        cx.notify();
    }
    pub(super) fn set_read_only(&mut self, readonly: bool, cx: &mut Context<Self>) {
        self.readonly = readonly;
        cx.notify();
    }
    pub(super) fn set_placeholder_text(&self, text: &str, window: &mut Window, cx: &mut App) {
        self.state
            .update(cx, |s, cx| s.set_placeholder(text.to_owned(), window, cx));
    }
}
impl EventEmitter<InputEvent> for InputField {}
impl Focusable for InputField {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.state.focus_handle(cx)
    }
}
impl Render for InputField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::component::v_flex()
            .w_full()
            .gap_1()
            .when(!self.label.is_empty(), |el| {
                el.child(Label::new(self.label.clone()).size(LabelSize::Small))
            })
            .child(
                Input::new(&self.state)
                    .readonly(self.readonly)
                    .tab_index(self.tab_index)
                    .aria_label(self.label.clone())
                    .when(self.code, |el| {
                        el.font_family(cx.theme().mono_font_family.clone())
                    }),
            )
            .when_some(self.error.clone(), |el, error| {
                el.child(Label::new(error).size(LabelSize::Small).color(Color::Error))
            })
    }
}
