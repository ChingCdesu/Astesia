use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

pub(super) struct UiAssets;

const ICONS: &[(&str, &[u8])] = &[
    (
        "icons/astesia/play-filled.svg",
        include_bytes!("../../icons/play-filled.svg"),
    ),
    (
        "icons/astesia/data-filter.svg",
        include_bytes!("../../icons/data-filter.svg"),
    ),
    (
        "icons/astesia/data-sort.svg",
        include_bytes!("../../icons/data-sort.svg"),
    ),
    (
        "icons/astesia/download.svg",
        include_bytes!("../../icons/download.svg"),
    ),
    (
        "icons/astesia/filter.svg",
        include_bytes!("../../icons/filter.svg"),
    ),
    (
        "icons/astesia/list_tree.svg",
        include_bytes!("../../icons/list_tree.svg"),
    ),
    (
        "icons/astesia/eraser.svg",
        include_bytes!("../../icons/eraser.svg"),
    ),
    (
        "icons/astesia/list_todo.svg",
        include_bytes!("../../icons/list_todo.svg"),
    ),
    (
        "icons/astesia/server.svg",
        include_bytes!("../../icons/server.svg"),
    ),
    (
        "icons/astesia/command.svg",
        include_bytes!("../../icons/command.svg"),
    ),
    (
        "icons/astesia/pencil.svg",
        include_bytes!("../../icons/pencil.svg"),
    ),
    (
        "icons/astesia/trash.svg",
        include_bytes!("../../icons/trash.svg"),
    ),
    (
        "icons/astesia/chart.svg",
        include_bytes!("../../icons/chart.svg"),
    ),
    (
        "icons/astesia/fit-window.svg",
        include_bytes!("../../icons/fit-window.svg"),
    ),
    (
        "icons/astesia/catalog-database.svg",
        include_bytes!("../../icons/catalog-database.svg"),
    ),
    (
        "icons/astesia/catalog-schema.svg",
        include_bytes!("../../icons/catalog-schema.svg"),
    ),
    (
        "icons/astesia/catalog-table.svg",
        include_bytes!("../../icons/catalog-table.svg"),
    ),
    (
        "icons/astesia/catalog-column.svg",
        include_bytes!("../../icons/catalog-column.svg"),
    ),
    (
        "icons/astesia/catalog-constraint.svg",
        include_bytes!("../../icons/catalog-constraint.svg"),
    ),
    (
        "icons/astesia/catalog-index.svg",
        include_bytes!("../../icons/catalog-index.svg"),
    ),
];

impl AssetSource for UiAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = ICONS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(
            ICONS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| (*name).into()),
        );
        Ok(paths)
    }
}
