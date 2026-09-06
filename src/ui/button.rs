use crate::ui::components::ButtonLike;
use gpui_kit::*;
pub(super) fn button_with_disabled_state(
    button: ButtonLike,
    disabled: bool,
    _: &mut Window,
    _: &mut App,
) -> AnyElement {
    button.disabled(disabled).into_any_element()
}
