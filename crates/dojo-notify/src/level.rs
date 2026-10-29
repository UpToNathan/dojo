//! The levels the volume and brightness keys report, read from `wpctl` and `brightnessctl`.

/// Output volume as a percentage, and whether it is muted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Volume {
    pub percent: u8,
    pub muted: bool,
}

/// `wpctl get-volume`: `Volume: 0.45` or `Volume: 0.45 [MUTED]`.
pub fn parse_volume(output: &str) -> Option<Volume> {
    let rest = output.trim().strip_prefix("Volume:")?;
    let level: f64 = rest.split_whitespace().next()?.parse().ok()?;
    if !level.is_finite() || level < 0.0 {
        return None;
    }
    Some(Volume { percent: (level * 100.0).round().min(100.0) as u8, muted: rest.contains("[MUTED]") })
}

/// The theme icon for a volume.
pub fn volume_icon(v: Volume) -> &'static str {
    match v {
        Volume { muted: true, .. } | Volume { percent: 0, .. } => "audio-volume-muted-symbolic",
        Volume { percent: 1..=33, .. } => "audio-volume-low-symbolic",
        Volume { percent: 34..=66, .. } => "audio-volume-medium-symbolic",
        _ => "audio-volume-high-symbolic",
    }
}

/// `brightnessctl -m`: `device,class,current,percent%,max`. Returns the percentage.
pub fn parse_brightness(output: &str) -> Option<u8> {
    let line = output.lines().find(|l| !l.trim().is_empty())?;
    let percent = line.split(',').nth(3)?.trim().strip_suffix('%')?;
    percent.parse().ok()
}
