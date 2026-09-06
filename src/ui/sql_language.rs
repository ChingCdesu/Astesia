use super::text_editor::Editor;
use gpui_kit::{Context, Window};
pub(super) fn editor(text: &str, window: &mut Window, cx: &mut Context<Editor>) -> Editor {
    Editor::code(text, "sql", window, cx)
}
