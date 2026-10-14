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
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    if unsafe { libc::localtime_r(&timestamp, local.as_mut_ptr()) }.is_null() {
        return "Clock unavailable".into();
    }
    format_local(&unsafe { local.assume_init() })
}

fn format_local(local: &libc::tm) -> String {
    let mut buffer = [0_u8; 80];
    let length = unsafe {
        libc::strftime(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            c"%a, %b %d  %H:%M".as_ptr(),
            local,
        )
    };
    String::from_utf8_lossy(&buffer[..length]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_shows_weekday_date_and_local_24_hour_time() {
        let mut local: libc::tm = unsafe { std::mem::zeroed() };
        local.tm_year = 126;
        local.tm_mon = 9;
        local.tm_mday = 14;
        local.tm_wday = 3;
        local.tm_hour = 13;
        local.tm_min = 5;
        assert_eq!(format_local(&local), "Wed, Oct 14  13:05");
    }
}
