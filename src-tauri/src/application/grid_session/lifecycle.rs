#[derive(Clone, Copy, Debug)]
pub(crate) enum GridSessionStatus<'a> {
    Idle,
    Loading,
    Saving,
    Ready,
    Failed { error: &'a str },
    SaveFailed { error: &'a str },
    Unavailable { reason: &'a str },
}

use super::GridPage;

#[derive(Debug)]
pub(super) enum GridState {
    Idle,
    Loading {
        generation: u64,
        page: Option<GridPage>,
    },
    Saving {
        generation: u64,
        page: GridPage,
    },
    Ready(GridPage),
    Failed {
        error: String,
        page: Option<GridPage>,
    },
    SaveFailed {
        error: String,
        page: GridPage,
    },
    Unavailable {
        reason: String,
        page: Option<GridPage>,
    },
}
