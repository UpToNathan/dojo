use std::time::Duration;

use gpui::{App, BorrowAppContext, Global};

pub struct Clock {
    pub label: String,
    _task: gpui::Task<()>,
}
impl Global for Clock {}

pub fn initialize(cx: &mut App) {
    let task = cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let label = local_label();
            let _ = cx.update(|cx| {
                if cx.global::<Clock>().label != label {
                    cx.update_global::<Clock, _>(|clock, _| clock.label = label);
                }
            });
        }
    });
    cx.set_global(Clock {
        label: local_label(),
        _task: task,
    });
}

fn local_label() -> String {
    let timestamp = unsafe { libc::time(std::ptr::null_mut()) };
    dojo::wall_clock::top_bar_label(timestamp)
}
