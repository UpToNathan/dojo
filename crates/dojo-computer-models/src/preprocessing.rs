//! Explicit image sizing plans. Caller performs resampling and records the actual
//! dimensions/filter; a sizing plan does not claim Pillow image-byte equivalence.
use crate::DecodeError as E;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResizePolicy {
    Area {
        minimum: u64,
        maximum: u64,
    },
    Factor {
        factor: u32,
        minimum: u64,
        maximum: u64,
    },
    Fit {
        maximum_width: u32,
        maximum_height: u32,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ResizePlan {
    pub width: u32,
    pub height: u32,
    pub restore_x: f64,
    pub restore_y: f64,
}
pub fn plan(width: u32, height: u32, policy: ResizePolicy) -> Result<ResizePlan, E> {
    if width == 0 || height == 0 || width > 32768 || height > 32768 {
        return Err(E::Invalid("image dimensions"));
    }
    let (w, h) = (f64::from(width), f64::from(height));
    let (rw, rh) = match policy {
        ResizePolicy::Fit {
            maximum_width,
            maximum_height,
        } => {
            if maximum_width == 0 || maximum_height == 0 {
                return Err(E::Invalid("fit bounds"));
            }
            let scale = (f64::from(maximum_width) / w)
                .min(f64::from(maximum_height) / h)
                .min(1.);
            ((w * scale).floor(), (h * scale).floor())
        }
        ResizePolicy::Area { minimum, maximum } => {
            bounds(minimum, maximum)?;
            let (mut rw, mut rh) = (w, h);
            if rw * rh > maximum as f64 {
                let scale = (maximum as f64 / (rw * rh)).sqrt();
                rw = (rw * scale).floor();
                rh = (rh * scale).floor();
            }
            if rw * rh < (minimum as f64) {
                let scale = (minimum as f64 / (rw * rh)).sqrt();
                rw = (rw * scale).ceil();
                rh = (rh * scale).ceil();
            }
            (rw, rh)
        }
        ResizePolicy::Factor {
            factor,
            minimum,
            maximum,
        } => {
            bounds(minimum, maximum)?;
            if factor == 0
                || factor > 32768
                || maximum < u64::from(factor) * u64::from(factor)
                || w.max(h) / w.min(h) > 200.
            {
                return Err(E::Invalid("factor/aspect"));
            }
            let f = f64::from(factor);
            let (mut rw, mut rh) = (
                (w / f).round_ties_even().max(1.) * f,
                (h / f).round_ties_even().max(1.) * f,
            );
            if rw * rh > maximum as f64 {
                let beta = (w * h / maximum as f64).sqrt();
                rw = (w / beta / f).floor() * f;
                rh = (h / beta / f).floor() * f;
            } else if rw * rh < (minimum as f64) {
                let beta = (minimum as f64 / (w * h)).sqrt();
                rw = (w * beta / f).ceil() * f;
                rh = (h * beta / f).ceil() * f;
            }
            (rw, rh)
        }
    };
    if rw < 1. || rh < 1. || rw > 32768. || rh > 32768. {
        return Err(E::Invalid("resized dimensions"));
    }
    Ok(ResizePlan {
        width: rw as u32,
        height: rh as u32,
        restore_x: w / rw,
        restore_y: h / rh,
    })
}
fn bounds(min: u64, max: u64) -> Result<(), E> {
    if min == 0 || max < min || max > 32768 * 32768 {
        Err(E::Invalid("pixel bounds"))
    } else {
        Ok(())
    }
}
