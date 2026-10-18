use std::borrow::Cow;
use std::fs;
use std::path::Path;

use anyhow::Context as _;
use gpui::App;

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

    #[test]
    fn chooses_battery_and_charging_glyphs_by_level() {
        assert_eq!(battery_glyph(Some(7), Some("Discharging")), '\u{f007a}');
        assert_eq!(battery_glyph(Some(7), Some("Charging")), '\u{f089c}');
        assert_eq!(battery_glyph(Some(100), Some("Full")), '\u{f0079}');
        assert_eq!(battery_glyph(None, None), '\u{f0091}');
    }
}
