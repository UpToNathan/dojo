use std::collections::BTreeMap;
use std::process::Command;

use anyhow::{Context as _, bail};
use serde::Deserialize;

use crate::geometry::{LEFT_PANEL_WIDTH, RIGHT_INSET};

const HORIZONTAL_RESERVED: i32 = LEFT_PANEL_WIDTH + RIGHT_INSET;
const MINIMUM_COLUMN_WIDTH: i32 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnWidthSnapshot {
    pub workspace_id: u64,
    pub column_index: usize,
    pub width: i32,
}

#[derive(Default)]
pub struct NiriLayout {
    saved_widths: Option<Vec<ColumnWidthSnapshot>>,
}

#[derive(Deserialize)]
struct WindowInfo {
    id: u64,
    workspace_id: Option<u64>,
    is_floating: bool,
    layout: WindowLayout,
}

#[derive(Deserialize)]
struct WindowLayout {
    pos_in_scrolling_layout: Option<(usize, usize)>,
    tile_size: (f64, f64),
}

impl NiriLayout {
    pub fn capture(&mut self) -> anyhow::Result<()> {
        if self.saved_widths.is_none() {
            self.saved_widths = Some(snapshot_columns()?);
        }
        Ok(())
    }

    pub fn enable(&mut self) -> anyhow::Result<()> {
        let Some(snapshot) = self.saved_widths.as_ref() else {
            bail!("Niri column widths must be captured before enabling the frame");
        };

        if let Err(error) = apply_widths(snapshot, WidthChange::Shrink) {
            if let Err(rollback_error) = apply_widths(snapshot, WidthChange::Restore) {
                eprintln!(
                    "Failed to roll back Niri columns after enable failure: {rollback_error:#}"
                );
            } else {
                self.saved_widths = None;
            }
            return Err(error).context("failed to shrink Niri columns for the desktop frame");
        }

        Ok(())
    }

    pub fn disable(&mut self) -> anyhow::Result<()> {
        let Some(snapshot) = self.saved_widths.as_ref() else {
            return Ok(());
        };

        if let Err(error) = apply_widths(snapshot, WidthChange::Restore) {
            if let Err(rollback_error) = apply_widths(snapshot, WidthChange::Shrink) {
                eprintln!(
                    "Failed to keep Niri columns inside the frame after restore failure: {rollback_error:#}"
                );
            }
            return Err(error).context("failed to restore Niri column widths");
        }

        self.saved_widths = None;
        Ok(())
    }
}

fn niri_windows() -> anyhow::Result<Vec<WindowInfo>> {
    let output = Command::new("niri")
        .args(["msg", "--json", "windows"])
        .output()
        .context("failed to run niri msg --json windows")?;
    if !output.status.success() {
        bail!(
            "niri msg --json windows failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(serde_json::from_slice(&output.stdout).context("invalid Niri windows JSON")?)
}

fn snapshot_columns() -> anyhow::Result<Vec<ColumnWidthSnapshot>> {
    Ok(columns_from_windows(niri_windows()?))
}

fn columns_from_windows(windows: Vec<WindowInfo>) -> Vec<ColumnWidthSnapshot> {
    let mut columns = BTreeMap::new();
    for window in windows {
        if window.is_floating {
            continue;
        }

        let (Some(workspace_id), Some((column_index, _))) =
            (window.workspace_id, window.layout.pos_in_scrolling_layout)
        else {
            continue;
        };

        columns
            .entry((workspace_id, column_index))
            .or_insert(ColumnWidthSnapshot {
                workspace_id,
                column_index,
                width: window.layout.tile_size.0.round() as i32,
            });
    }
    columns.into_values().collect()
}

#[derive(Clone, Copy)]
enum WidthChange {
    Shrink,
    Restore,
}

fn apply_widths(snapshots: &[ColumnWidthSnapshot], change: WidthChange) -> anyhow::Result<()> {
    let windows = niri_windows()?;

    for snapshot in snapshots {
        let Some(window) = windows.iter().find(|window| {
            !window.is_floating
                && window.workspace_id == Some(snapshot.workspace_id)
                && window
                    .layout
                    .pos_in_scrolling_layout
                    .is_some_and(|(column_index, _)| column_index == snapshot.column_index)
        }) else {
            continue;
        };

        let width = match change {
            WidthChange::Shrink => (snapshot.width - HORIZONTAL_RESERVED).max(MINIMUM_COLUMN_WIDTH),
            WidthChange::Restore => snapshot.width,
        };
        let current_width = window.layout.tile_size.0.round() as i32;
        if matches!(change, WidthChange::Shrink) && !needs_shrink(current_width, width) {
            continue;
        }
        set_window_width(window.id, width)?;
    }

    Ok(())
}

fn needs_shrink(current_width: i32, target_width: i32) -> bool {
    current_width > target_width
}

fn set_window_width(window_id: u64, width: i32) -> anyhow::Result<()> {
    let window_id = window_id.to_string();
    let width = width.to_string();
    let output = Command::new("niri")
        .args([
            "msg",
            "action",
            "set-window-width",
            "--id",
            &window_id,
            &width,
        ])
        .output()
        .context("failed to run niri msg action set-window-width")?;
    if !output.status.success() {
        bail!(
            "niri failed to set window {window_id} width: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_one_width_per_tiled_column() {
        let windows: Vec<WindowInfo> = serde_json::from_str(
            r#"[
                {"id": 1, "workspace_id": 4, "is_floating": false, "layout": {"pos_in_scrolling_layout": [1, 1], "tile_size": [900.0, 500.0]}},
                {"id": 2, "workspace_id": 4, "is_floating": false, "layout": {"pos_in_scrolling_layout": [1, 2], "tile_size": [900.0, 240.0]}},
                {"id": 3, "workspace_id": 4, "is_floating": false, "layout": {"pos_in_scrolling_layout": [2, 1], "tile_size": [600.0, 500.0]}},
                {"id": 4, "workspace_id": 4, "is_floating": true, "layout": {"pos_in_scrolling_layout": null, "tile_size": [300.0, 300.0]}}
            ]"#,
        )
        .expect("test Niri window data should parse");

        assert_eq!(
            columns_from_windows(windows),
            vec![
                ColumnWidthSnapshot {
                    workspace_id: 4,
                    column_index: 1,
                    width: 900,
                },
                ColumnWidthSnapshot {
                    workspace_id: 4,
                    column_index: 2,
                    width: 600,
                },
            ]
        );
    }

    #[test]
    fn left_panel_and_right_inset_are_removed_from_each_tiled_column() {
        assert_eq!((1888 - HORIZONTAL_RESERVED).max(MINIMUM_COLUMN_WIDTH), 1812);
        assert_eq!(
            (130 - HORIZONTAL_RESERVED).max(MINIMUM_COLUMN_WIDTH),
            MINIMUM_COLUMN_WIDTH
        );
    }

    #[test]
    fn does_not_shrink_columns_niri_already_reflowed() {
        assert!(!needs_shrink(1864, 1864));
        assert!(needs_shrink(1888, 1864));
    }
}
