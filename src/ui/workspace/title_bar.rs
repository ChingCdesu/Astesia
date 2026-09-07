use gpui_kit::component::{ActiveTheme, TitleBar};
use gpui_kit::{prelude::*, *};
#[derive(Default)]
pub(super) struct AstesiaTitleBar;
impl Render for AstesiaTitleBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        TitleBar::new().bg(cx.theme().title_bar)
    }
}
