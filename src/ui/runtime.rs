use gpui_kit::{App, Global};
use std::future::Future;
struct Runtime(tokio::runtime::Runtime);
impl Global for Runtime {}
pub(super) fn init(cx: &mut App) {
    cx.set_global(Runtime(
        tokio::runtime::Runtime::new().expect("failed to create database task runtime"),
    ));
}
pub(super) fn spawn<F>(cx: &App, future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    cx.global::<Runtime>().0.spawn(future)
}
