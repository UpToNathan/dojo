#[cfg(not(target_os = "linux"))]
compile_error!("dojo-speech currently supports Linux only");

use sha2::Digest;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[path = "../../../vendor/hex/src/audio.rs"]
#[allow(dead_code)]
mod audio;
#[path = "../../../vendor/hex/src/dictation.rs"]
#[allow(dead_code)]
mod dictation;
#[path = "../../../vendor/hex/src/gguf_session.rs"]
#[allow(dead_code)]
mod gguf_session;
#[path = "../../../vendor/hex/src/linux_transcriber.rs"]
#[allow(dead_code)]
mod linux_transcriber;
#[path = "../../../vendor/hex/src/spoken_text.rs"]
#[allow(dead_code)]
mod spoken_text;
#[path = "../../../vendor/hex/src/transcription_models.rs"]
#[allow(dead_code)]
mod transcription_models;

mod app_paths;
mod capture;
mod clip;

pub use capture::Capture;
pub use clip::AudioClip;
use transcription_models::{ModelRuntime, TranscriptionModelId, TranscriptionSelection};

pub struct SpeechError(pub(crate) String);

impl fmt::Display for SpeechError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl fmt::Debug for SpeechError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("SpeechError").field(&self.0).finish()
    }
}

impl std::error::Error for SpeechError {}

pub(crate) fn error(error: impl fmt::Display) -> SpeechError {
    SpeechError(error.to_string())
}

pub type Result<T> = std::result::Result<T, SpeechError>;

pub struct Selection(TranscriptionSelection);

impl Default for Selection {
    fn default() -> Self {
        Self(TranscriptionSelection::default())
    }
}

impl Selection {
    pub fn new(model: &str, language: &str) -> Result<Self> {
        let model = TranscriptionModelId::from_str(model).map_err(error)?;
        let selection = TranscriptionSelection {
            model,
            language: language.to_owned(),
            recognition_hints: String::new(),
        };
        transcription_models::validate(&selection).map_err(error)?;
        Ok(Self(selection))
    }

    pub fn model(&self) -> &str {
        self.0.model.as_str()
    }

    pub fn language(&self) -> &str {
        &self.0.language
    }
}

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_canceled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn check(&self) -> Result<()> {
        if self.is_canceled() {
            Err(error("operation canceled"))
        } else {
            Ok(())
        }
    }
}

pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub bytes: u64,
    pub installed: bool,
}

pub fn models() -> Result<Vec<ModelInfo>> {
    transcription_models::MODELS
        .iter()
        .filter_map(|definition| match definition.runtime {
            ModelRuntime::Gguf(artifact) => Some((definition, artifact)),
            ModelRuntime::AppleSpeech => None,
        })
        .map(|(definition, artifact)| {
            let path = transcription_models::model_path(definition).map_err(error)?;
            let installed = std::fs::metadata(path)
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() == artifact.bytes);
            Ok(ModelInfo {
                id: definition.id.as_str().to_owned(),
                name: definition.name.to_owned(),
                bytes: artifact.bytes,
                installed,
            })
        })
        .collect()
}

pub fn microphone_devices() -> Result<Vec<String>> {
    audio::input_device_names().map_err(error)
}

pub fn install_model(selection: &Selection, cancellation: &Cancellation) -> Result<PathBuf> {
    cancellation.check()?;
    let definition = transcription_models::validate(&selection.0).map_err(error)?;
    let downloaded = AtomicU64::new(0);
    let path =
        transcription_models::download_with_progress(definition, &cancellation.0, &downloaded)
            .map_err(error)?;
    cancellation.check()?;
    Ok(path)
}

pub struct Transcriber(linux_transcriber::LinuxTranscriber);

impl Transcriber {
    pub fn load(selection: &Selection, cancellation: &Cancellation) -> Result<Self> {
        cancellation.check()?;
        let definition = transcription_models::validate(&selection.0).map_err(error)?;
        let ModelRuntime::Gguf(artifact) = definition.runtime else {
            return Err(error("only local GGUF models are supported"));
        };
        let path = transcription_models::model_path(definition).map_err(error)?;
        verify_artifact(&path, artifact.bytes, artifact.sha256, cancellation)?;
        cancellation.check()?;
        let transcriber = linux_transcriber::LinuxTranscriber::load(&selection.0).map_err(error)?;
        cancellation.check()?;
        Ok(Self(transcriber))
    }

    pub fn transcribe(&mut self, clip: AudioClip, cancellation: &Cancellation) -> Result<String> {
        cancellation.check()?;
        let mut samples = clip.into_samples();
        dictation::pad_for_parakeet(&mut samples);
        let text = self.0.transcribe(&samples).map_err(error)?;
        cancellation.check()?;
        Ok(text)
    }

    pub fn device_name(&self) -> &str {
        self.0.device_label()
    }
}

fn verify_artifact(
    path: &std::path::Path,
    expected_bytes: u64,
    expected_sha256: &str,
    cancellation: &Cancellation,
) -> Result<()> {
    let metadata = std::fs::metadata(path).map_err(error)?;
    if !metadata.is_file() || metadata.len() != expected_bytes {
        return Err(error(format!(
            "model artifact has an unexpected size: {}",
            path.display()
        )));
    }
    let mut file = File::open(path).map_err(error)?;
    let mut digest = sha2::Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        cancellation.check()?;
        let count = file.read(&mut buffer).map_err(error)?;
        if count == 0 {
            break;
        }
        sha2::Digest::update(&mut digest, &buffer[..count]);
    }
    let actual: String = sha2::Digest::finalize(digest)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual != expected_sha256 {
        return Err(error(format!(
            "model artifact checksum mismatch: {}",
            path.display()
        )));
    }
    cancellation.check()
}
