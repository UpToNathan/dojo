use std::borrow::Cow;
use std::fs;
use std::path::Path;

use anyhow::Context as _;
use gpui::App;

use crate::notify::{Notice, Urgency};

pub const FONT_FAMILY: &str = "VictorMono Nerd Font Mono";

pub fn font_features() -> gpui::FontFeatures {
    gpui::FontFeatures(std::sync::Arc::new(vec![("ss01".into(), 1)]))
}

pub fn font() -> gpui::Font {
    gpui::Font {
        features: font_features(),
        ..gpui::font(FONT_FAMILY)
    }
}

pub fn register_font(cx: &App) -> anyhow::Result<()> {
    let font: &'static [u8] = include_bytes!("../assets/fonts/VictorMonoNerdFontMono-Regular.ttf");
    cx.text_system()
        .add_fonts(vec![Cow::Borrowed(font)])
        .context("registering the bundled Victor Mono Nerd Font")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PowerStatus {
    pub capacity: Option<u8>,
    pub battery_status: Option<String>,
}

impl PowerStatus {
    /// False on machines with no battery, such as desktops.
    pub fn has_battery(&self) -> bool {
        self.capacity.is_some() || self.battery_status.is_some()
    }
}

/// Percent at or below which a discharging battery is "low".
const LOW_AT: u8 = 20;
/// Percent at or below which a discharging battery is "critically low".
const CRITICAL_AT: u8 = 10;
/// Percent above which both low notices are armed again.
const REARM_ABOVE: u8 = 25;

/// Watches successive battery readings and says which Notices they call for. It holds no GPUI or
/// host state, so the host-state loop feeds it each new reading and posts what comes back.
#[derive(Debug, Default)]
pub struct BatteryWatch {
    previous: Option<PowerStatus>,
    low_sent: bool,
    critical_sent: bool,
}

impl BatteryWatch {
    /// Takes the next reading. The first reading with a battery is silent: it only sets the
    /// baseline, and a battery already below a threshold counts as having fired it. Readings
    /// without a battery send nothing and leave the baseline alone.
    pub fn observe(&mut self, next: &PowerStatus) -> Vec<Notice> {
        if !next.has_battery() {
            return Vec::new();
        }
        let status = next.battery_status.as_deref();
        let discharging = status == Some("Discharging");
        let below = |limit: u8| next.capacity.is_some_and(|capacity| capacity <= limit);

        let mut notices = Vec::new();
        let first = self.previous.is_none();

        if let Some(previous) = &self.previous {
            match (previous.battery_status.as_deref(), status) {
                (Some("Discharging"), Some("Charging")) => {
                    notices.push(battery_notice("Charging", Urgency::Low, "battery-charging"));
                }
                (Some("Charging" | "Full"), Some("Discharging")) => {
                    notices.push(battery_notice("On battery", Urgency::Low, "battery"));
                }
                (Some("Charging"), Some("Full")) => {
                    notices.push(battery_notice(
                        "Fully charged",
                        Urgency::Low,
                        "battery-full-charged",
                    ));
                }
                _ => {}
            }
        }

        if status == Some("Charging") || next.capacity.is_some_and(|c| c > REARM_ABOVE) {
            self.low_sent = false;
            self.critical_sent = false;
        } else if discharging {
            if below(CRITICAL_AT) && !self.critical_sent {
                self.critical_sent = true;
                self.low_sent = true;
                if !first {
                    let capacity = next.capacity.unwrap_or_default();
                    notices.push(
                        battery_notice(
                            "Battery critically low",
                            Urgency::Critical,
                            "battery-caution",
                        )
                        .body(format!("{capacity}% left. Plug in a charger now.")),
                    );
                }
            } else if below(LOW_AT) && !self.low_sent {
                self.low_sent = true;
                if !first {
                    let capacity = next.capacity.unwrap_or_default();
                    notices.push(
                        battery_notice("Battery low", Urgency::Normal, "battery-low")
                            .body(format!("{capacity}% left.")),
                    );
                }
            }
        }

        self.previous = Some(next.clone());
        notices
    }
}

fn battery_notice(summary: &str, urgency: Urgency, icon: &str) -> Notice {
    Notice::new("battery", summary).urgency(urgency).icon(icon)
}

pub fn read_status() -> PowerStatus {
    let (capacity, battery_status) = read_battery(Path::new("/sys/class/power_supply"));

    PowerStatus {
        capacity,
        battery_status,
    }
}

pub fn battery_glyph(capacity: Option<u8>, status: Option<&str>) -> char {
    let Some(capacity) = capacity else {
        return '\u{f0091}'; // nf-md-battery_unknown
    };

    let level = match capacity {
        0..=4 => 0,
        5..=14 => 10,
        15..=24 => 20,
        25..=34 => 30,
        35..=44 => 40,
        45..=54 => 50,
        55..=64 => 60,
        65..=74 => 70,
        75..=84 => 80,
        85..=94 => 90,
        _ => 100,
    };

    if status == Some("Charging") {
        match level {
            0 | 10 => '\u{f089c}',
            20 => '\u{f0086}',
            30 => '\u{f0087}',
            40 => '\u{f0088}',
            50 => '\u{f089d}',
            60 => '\u{f0089}',
            70 => '\u{f089e}',
            80 => '\u{f008a}',
            90 => '\u{f008b}',
            _ => '\u{f0085}',
        }
    } else {
        match level {
            0 => '\u{f008e}',
            10 => '\u{f007a}',
            20 => '\u{f007b}',
            30 => '\u{f007c}',
            40 => '\u{f007d}',
            50 => '\u{f007e}',
            60 => '\u{f007f}',
            70 => '\u{f0080}',
            80 => '\u{f0081}',
            90 => '\u{f0082}',
            _ => '\u{f0079}',
        }
    }
}

fn read_battery(root: &Path) -> (Option<u8>, Option<String>) {
    let Ok(entries) = fs::read_dir(root) else {
        return (None, None);
    };

    let mut capacities = Vec::new();
    let mut statuses = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if read_trimmed(&path.join("type")).as_deref() != Some("Battery") {
            continue;
        }
        if read_trimmed(&path.join("scope")).as_deref() == Some("Device") {
            continue;
        }
        if let Some(value) = read_trimmed(&path.join("capacity")).and_then(|s| s.parse::<u8>().ok())
        {
            capacities.push(value.min(100));
        }
        if let Some(status) = read_trimmed(&path.join("status")) {
            statuses.push(status);
        }
    }

    let capacity = (!capacities.is_empty()).then(|| {
        (capacities
            .iter()
            .map(|value| u16::from(*value))
            .sum::<u16>()
            / capacities.len() as u16) as u8
    });
    let status = if statuses.iter().any(|status| status == "Charging") {
        Some("Charging".to_string())
    } else if statuses.iter().all(|status| status == "Full") && !statuses.is_empty() {
        Some("Full".to_string())
    } else if statuses.iter().any(|status| status == "Discharging") {
        Some("Discharging".to_string())
    } else {
        statuses.into_iter().next()
    };
    (capacity, status)
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peripheral_battery_does_not_change_laptop_capacity_or_status() {
        let root = tempfile::tempdir().unwrap();
        for (name, scope, capacity, status) in [
            ("BAT0", "System", "100", "Full"),
            ("ps-controller", "Device", "64", "Charging"),
        ] {
            let battery = root.path().join(name);
            fs::create_dir(&battery).unwrap();
            for (field, value) in [
                ("type", "Battery"),
                ("scope", scope),
                ("capacity", capacity),
                ("status", status),
            ] {
                fs::write(battery.join(field), value).unwrap();
            }
        }
        assert_eq!(read_battery(root.path()), (Some(100), Some("Full".into())));
    }

    #[test]
    fn reads_battery_capacity_and_charging_state() {
        let root = std::env::temp_dir().join(format!("dojo-power-test-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let battery = root.join("BAT0");
        fs::create_dir(&battery).unwrap();
        fs::write(battery.join("type"), "Battery\n").unwrap();
        fs::write(battery.join("capacity"), "84\n").unwrap();
        fs::write(battery.join("status"), "Charging\n").unwrap();

        assert_eq!(
            read_battery(&root),
            (Some(84), Some("Charging".to_string()))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_machines_with_a_battery_report_one() {
        assert!(!PowerStatus::default().has_battery());
        let battery = PowerStatus {
            capacity: Some(80),
            battery_status: Some("Discharging".into()),
        };
        assert!(battery.has_battery());
        let root = tempfile::tempdir().unwrap();
        assert_eq!(read_battery(root.path()), (None, None));
    }

    fn reading(capacity: u8, status: &str) -> PowerStatus {
        PowerStatus {
            capacity: Some(capacity),
            battery_status: Some(status.into()),
        }
    }

    fn summaries(notices: &[Notice]) -> Vec<(&str, Urgency)> {
        notices
            .iter()
            .map(|notice| (notice.summary.as_str(), notice.urgency))
            .collect()
    }

    #[test]
    fn battery_notices_all_share_one_kind() {
        let mut watch = BatteryWatch::default();
        watch.observe(&reading(30, "Discharging"));
        let notices = watch.observe(&reading(8, "Discharging"));
        assert!(notices.iter().all(|notice| notice.kind == "battery"));
    }

    #[test]
    fn first_reading_is_silent_even_when_low_or_plugged_in() {
        for first in [
            reading(5, "Discharging"),
            reading(15, "Discharging"),
            reading(50, "Charging"),
            reading(100, "Full"),
        ] {
            assert!(BatteryWatch::default().observe(&first).is_empty());
        }
    }

    #[test]
    fn low_then_critical_fire_once_each_on_a_fall() {
        let mut watch = BatteryWatch::default();
        assert!(watch.observe(&reading(22, "Discharging")).is_empty());
        assert!(watch.observe(&reading(21, "Discharging")).is_empty());
        let low = watch.observe(&reading(20, "Discharging"));
        assert_eq!(summaries(&low), [("Battery low", Urgency::Normal)]);
        assert!(watch.observe(&reading(19, "Discharging")).is_empty());
        assert!(watch.observe(&reading(11, "Discharging")).is_empty());
        let critical = watch.observe(&reading(10, "Discharging"));
        assert_eq!(
            summaries(&critical),
            [("Battery critically low", Urgency::Critical)]
        );
        assert!(!critical[0].body.is_empty());
        assert!(watch.observe(&reading(9, "Discharging")).is_empty());
        assert!(watch.observe(&reading(9, "Discharging")).is_empty());
        assert!(watch.observe(&reading(1, "Discharging")).is_empty());
    }

    #[test]
    fn jumping_past_both_thresholds_sends_only_the_critical_notice() {
        let mut watch = BatteryWatch::default();
        watch.observe(&reading(40, "Discharging"));
        let notices = watch.observe(&reading(8, "Discharging"));
        assert_eq!(
            summaries(&notices),
            [("Battery critically low", Urgency::Critical)]
        );
        assert!(watch.observe(&reading(7, "Discharging")).is_empty());
    }

    #[test]
    fn a_first_reading_below_a_threshold_does_not_fire_it_later() {
        let mut watch = BatteryWatch::default();
        watch.observe(&reading(15, "Discharging"));
        assert!(watch.observe(&reading(14, "Discharging")).is_empty());
        let critical = watch.observe(&reading(10, "Discharging"));
        assert_eq!(critical.len(), 1);
        assert_eq!(critical[0].urgency, Urgency::Critical);
    }

    #[test]
    fn staying_between_twenty_and_twenty_five_does_not_rearm() {
        let mut watch = BatteryWatch::default();
        watch.observe(&reading(30, "Discharging"));
        assert_eq!(watch.observe(&reading(19, "Discharging")).len(), 1);
        assert!(watch.observe(&reading(25, "Discharging")).is_empty());
        assert!(watch.observe(&reading(19, "Discharging")).is_empty());
    }

    #[test]
    fn rearms_above_twenty_five_percent() {
        let mut watch = BatteryWatch::default();
        watch.observe(&reading(30, "Discharging"));
        assert_eq!(watch.observe(&reading(9, "Discharging")).len(), 1);
        assert!(watch.observe(&reading(26, "Discharging")).is_empty());
        let low = watch.observe(&reading(20, "Discharging"));
        assert_eq!(summaries(&low), [("Battery low", Urgency::Normal)]);
        let critical = watch.observe(&reading(10, "Discharging"));
        assert_eq!(
            summaries(&critical),
            [("Battery critically low", Urgency::Critical)]
        );
    }

    #[test]
    fn rearms_after_charging() {
        let mut watch = BatteryWatch::default();
        watch.observe(&reading(30, "Discharging"));
        assert_eq!(watch.observe(&reading(9, "Discharging")).len(), 1);
        let plugged = watch.observe(&reading(9, "Charging"));
        assert_eq!(summaries(&plugged), [("Charging", Urgency::Low)]);
        let unplugged = watch.observe(&reading(12, "Discharging"));
        assert_eq!(
            summaries(&unplugged),
            [
                ("On battery", Urgency::Low),
                ("Battery low", Urgency::Normal)
            ]
        );
    }

    #[test]
    fn plugging_and_unplugging_send_low_notices_once() {
        let mut watch = BatteryWatch::default();
        watch.observe(&reading(60, "Discharging"));
        let plugged = watch.observe(&reading(60, "Charging"));
        assert_eq!(summaries(&plugged), [("Charging", Urgency::Low)]);
        assert!(watch.observe(&reading(61, "Charging")).is_empty());
        let unplugged = watch.observe(&reading(61, "Discharging"));
        assert_eq!(summaries(&unplugged), [("On battery", Urgency::Low)]);
        assert!(watch.observe(&reading(60, "Discharging")).is_empty());
    }

    #[test]
    fn fully_charged_only_after_charging() {
        let mut watch = BatteryWatch::default();
        watch.observe(&reading(98, "Charging"));
        let full = watch.observe(&reading(100, "Full"));
        assert_eq!(summaries(&full), [("Fully charged", Urgency::Low)]);
        assert!(watch.observe(&reading(100, "Full")).is_empty());
        let unplugged = watch.observe(&reading(99, "Discharging"));
        assert_eq!(summaries(&unplugged), [("On battery", Urgency::Low)]);

        let mut watch = BatteryWatch::default();
        watch.observe(&reading(100, "Full"));
        assert!(watch.observe(&reading(100, "Full")).is_empty());
    }

    #[test]
    fn machines_without_a_battery_send_nothing() {
        let mut watch = BatteryWatch::default();
        for _ in 0..3 {
            assert!(watch.observe(&PowerStatus::default()).is_empty());
        }
    }

    #[test]
    fn a_reading_that_loses_the_battery_keeps_the_baseline() {
        let mut watch = BatteryWatch::default();
        watch.observe(&reading(60, "Discharging"));
        assert!(watch.observe(&PowerStatus::default()).is_empty());
        let plugged = watch.observe(&reading(60, "Charging"));
        assert_eq!(summaries(&plugged), [("Charging", Urgency::Low)]);
    }

    #[test]
    fn unknown_statuses_and_missing_capacity_send_nothing() {
        let mut watch = BatteryWatch::default();
        watch.observe(&reading(50, "Discharging"));
        for odd in ["Not charging", "Unknown", "", "charging", "Mystery"] {
            assert!(watch.observe(&reading(5, odd)).is_empty(), "{odd}");
        }
        let no_capacity = PowerStatus {
            capacity: None,
            battery_status: Some("Discharging".into()),
        };
        assert!(watch.observe(&no_capacity).is_empty());
        let no_status = PowerStatus {
            capacity: Some(5),
            battery_status: None,
        };
        assert!(watch.observe(&no_status).is_empty());
    }

    #[test]
    fn chooses_battery_and_charging_glyphs_by_level() {
        assert_eq!(battery_glyph(Some(7), Some("Discharging")), '\u{f007a}');
        assert_eq!(battery_glyph(Some(7), Some("Charging")), '\u{f089c}');
        assert_eq!(battery_glyph(Some(100), Some("Full")), '\u{f0079}');
        assert_eq!(battery_glyph(None, None), '\u{f0091}');
    }
}
