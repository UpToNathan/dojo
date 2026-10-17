use std::ffi::CStr;

pub fn top_bar_label(unix: i64) -> String {
    local_tm(unix).map_or_else(|| "Clock unavailable".into(), |tm| top_bar_text(&tm))
}

pub fn dial_time(unix: i64) -> String {
    local_tm(unix).map_or_else(String::new, |tm| dial_text(&tm))
}

pub fn date_line(unix: i64) -> String {
    local_tm(unix).map_or_else(String::new, |tm| date_text(&tm))
}

fn local_tm(unix: i64) -> Option<libc::tm> {
    let timestamp: libc::time_t = unix;
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    if unsafe { libc::localtime_r(&timestamp, local.as_mut_ptr()) }.is_null() {
        return None;
    }
    Some(unsafe { local.assume_init() })
}

fn top_bar_text(tm: &libc::tm) -> String {
    format_tm(tm, c"%a, %b %d  %H:%M")
}

fn dial_text(tm: &libc::tm) -> String {
    format_tm(tm, c"%H:%M")
}

fn date_text(tm: &libc::tm) -> String {
    format_tm(tm, c"%A %-d %B")
}

fn format_tm(local: &libc::tm, pattern: &CStr) -> String {
    let mut buffer = [0_u8; 80];
    let length = unsafe {
        libc::strftime(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            pattern.as_ptr(),
            local,
        )
    };
    String::from_utf8_lossy(&buffer[..length]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Wed 2026-10-14 13:05 UTC. Formatting goes through gmtime so the
    // assertions do not depend on the machine's time zone.
    const FIXED_UNIX: i64 = 1_791_983_100;

    fn utc(unix: i64) -> libc::tm {
        let timestamp: libc::time_t = unix;
        let mut broken = std::mem::MaybeUninit::<libc::tm>::uninit();
        assert!(!unsafe { libc::gmtime_r(&timestamp, broken.as_mut_ptr()) }.is_null());
        unsafe { broken.assume_init() }
    }

    #[test]
    fn top_bar_shows_weekday_date_and_24_hour_time() {
        assert_eq!(top_bar_text(&utc(FIXED_UNIX)), "Wed, Oct 14  13:05");
    }

    #[test]
    fn dial_shows_24_hour_time() {
        assert_eq!(dial_text(&utc(FIXED_UNIX)), "13:05");
    }

    #[test]
    fn date_line_has_full_names_and_unpadded_day() {
        assert_eq!(
            date_text(&utc(FIXED_UNIX + 3 * 86_400)),
            "Saturday 17 October"
        );
        assert_eq!(
            date_text(&utc(FIXED_UNIX - 10 * 86_400)),
            "Sunday 4 October"
        );
    }
}
