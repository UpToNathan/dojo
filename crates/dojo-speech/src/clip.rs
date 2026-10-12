use std::io::{Read, Seek};

use crate::{Result, SpeechError, dictation, error};

/// Validated mono PCM at the engine's 16 kHz sample rate.
pub struct AudioClip {
    samples: Vec<f32>,
}

impl AudioClip {
    pub fn from_pcm(samples: Vec<f32>, sample_rate: u32, channels: u16) -> Result<Self> {
        if !(8_000..=192_000).contains(&sample_rate) || !(1..=32).contains(&channels) {
            return Err(SpeechError(
                "unsupported sample rate or channel count".into(),
            ));
        }
        let channels = usize::from(channels);
        if samples.is_empty() || samples.len() % channels != 0 {
            return Err(SpeechError(
                "audio must contain complete, nonempty frames".into(),
            ));
        }
        if samples
            .iter()
            .any(|sample| !sample.is_finite() || sample.abs() > 1.0)
        {
            return Err(SpeechError(
                "PCM samples must be finite and between -1 and 1".into(),
            ));
        }
        let mono = if channels == 1 {
            samples
        } else {
            samples
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                .collect()
        };
        let samples = if sample_rate == 16_000 {
            mono
        } else {
            dictation::try_resample_for_parakeet(&mono, sample_rate).map_err(error)?
        };
        Ok(Self { samples })
    }

    pub fn from_wav(reader: impl Read + Seek) -> Result<Self> {
        let mut reader = hound::WavReader::new(reader).map_err(error)?;
        let spec = reader.spec();
        if !(8_000..=192_000).contains(&spec.sample_rate) || !(1..=32).contains(&spec.channels) {
            return Err(SpeechError(
                "unsupported WAV sample rate or channel count".into(),
            ));
        }
        // Bound file admission before allocating from untrusted WAV metadata.
        let frames = u64::from(reader.duration());
        if frames == 0 || frames > u64::from(spec.sample_rate) * 60 * 30 {
            return Err(SpeechError(
                "WAV must contain between one frame and 30 minutes of audio".into(),
            ));
        }
        let normalized_frames = frames * 16_000 / u64::from(spec.sample_rate);
        if frames * u64::from(spec.channels) > 28_800_000 || normalized_frames > 28_800_000 {
            return Err(SpeechError(
                "WAV exceeds the speech audio memory budget".into(),
            ));
        }
        let scale = 2_f32.powi(i32::from(spec.bits_per_sample) - 1);
        let samples = match (spec.sample_format, spec.bits_per_sample) {
            (hound::SampleFormat::Float, 32) => reader
                .samples::<f32>()
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(error)?,
            (hound::SampleFormat::Int, 1..=8) => reader
                .samples::<i8>()
                .map(|sample| sample.map(|v| f32::from(v) / scale))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(error)?,
            (hound::SampleFormat::Int, 9..=16) => reader
                .samples::<i16>()
                .map(|sample| sample.map(|v| f32::from(v) / scale))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(error)?,
            (hound::SampleFormat::Int, 17..=32) => reader
                .samples::<i32>()
                .map(|sample| sample.map(|v| v as f32 / scale))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(error)?,
            _ => return Err(SpeechError("unsupported WAV sample format".into())),
        };
        Self::from_pcm(samples, spec.sample_rate, spec.channels)
    }

    pub fn duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(self.samples.len() as f64 / 16_000.0)
    }

    pub(crate) fn into_samples(self) -> Vec<f32> {
        self.samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_rate_pcm_preserves_samples_and_duration() {
        let clip = AudioClip::from_pcm(vec![-1.0, -0.5, 0.0, 0.5, 1.0], 16_000, 1).unwrap();
        assert_eq!(clip.duration(), std::time::Duration::from_nanos(312_500));
        assert_eq!(clip.into_samples(), vec![-1.0, -0.5, 0.0, 0.5, 1.0]);
    }

    #[test]
    fn native_rate_stereo_downmixes_without_resampling() {
        let clip = AudioClip::from_pcm(vec![1.0, -1.0, 0.5, 0.5], 16_000, 2).unwrap();
        assert_eq!(clip.into_samples(), vec![0.0, 0.5]);
    }

    #[test]
    fn native_rate_shortcut_retains_pcm_validation() {
        for samples in [vec![], vec![f32::NAN], vec![f32::INFINITY], vec![1.01]] {
            assert!(AudioClip::from_pcm(samples, 16_000, 1).is_err());
        }
        assert!(AudioClip::from_pcm(vec![0.0], 16_000, 2).is_err());
        assert!(AudioClip::from_pcm(vec![0.0], 16_000, 0).is_err());
    }
}
