use gpui_kit::component::{label::Label, TitleBar};
use gpui_kit::{prelude::*, *};
#[derive(Default)]
pub(super) struct AstesiaTitleBar;
impl Render for AstesiaTitleBar {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        TitleBar::new().child(Label::new("Astesia"))
    }
}
