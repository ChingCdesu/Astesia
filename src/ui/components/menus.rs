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
