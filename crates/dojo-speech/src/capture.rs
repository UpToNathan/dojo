use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::audio::{AudioInput, AudioInputEvent};
use crate::{AudioClip, Result, error};

const READ_INTERVAL: Duration = Duration::from_millis(25);

pub struct Capture {
    stop: Option<Sender<bool>>,
    worker: Option<JoinHandle<Result<Option<AudioClip>>>>,
    device_name: String,
}

impl Capture {
    pub fn start(device: Option<&str>) -> Result<Self> {
        let device = device.map(str::to_owned);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let (stop_sender, stop_receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("dojo-speech-capture".into())
            .spawn(move || capture_worker(device.as_deref(), stop_receiver, ready_sender))
            .map_err(error)?;

        match ready_receiver.recv() {
            Ok(Ok(device_name)) => Ok(Self {
                stop: Some(stop_sender),
                worker: Some(worker),
                device_name,
            }),
            Ok(Err(message)) => {
                let _ = worker.join();
                Err(error(message))
            }
            Err(_) => {
                let _ = worker.join();
                Err(error("microphone capture stopped before it became ready"))
            }
        }
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn finish(mut self) -> Result<AudioClip> {
        self.stop_worker(true);
        self.join_worker()?
            .ok_or_else(|| error("capture ended without audio"))
    }

    pub fn cancel(mut self) {
        self.stop_worker(false);
        let _ = self.join_worker();
    }

    fn stop_worker(&mut self, keep_audio: bool) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(keep_audio);
        }
    }

    fn join_worker(&mut self) -> Result<Option<AudioClip>> {
        self.worker
            .take()
            .ok_or_else(|| error("microphone capture was already stopped"))?
            .join()
            .map_err(|_| error("microphone capture worker panicked"))?
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop_worker(false);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn capture_worker(
    device: Option<&str>,
    stop: Receiver<bool>,
    ready: mpsc::SyncSender<std::result::Result<String, String>>,
) -> Result<Option<AudioClip>> {
    let input = match device {
        Some(device) => AudioInput::open_matching(device),
        None => AudioInput::open(&[]).or_else(|default_error| {
            for name in crate::audio::input_device_names()? {
                if name.to_lowercase().contains("discard") {
                    continue;
                }
                if let Ok(input) = AudioInput::open_named(&name) {
                    return Ok(input);
                }
            }
            Err(default_error)
        }),
    }
    .map_err(|open_error| {
        let message = format!("could not open microphone: {open_error:#}");
        let _ = ready.send(Err(message.clone()));
        error(message)
    })?;

    let device_name = input.device_name.clone();
    let sample_rate = input.sample_rate;
    ready
        .send(Ok(device_name))
        .map_err(|_| error("microphone capture caller disconnected during startup"))?;

    let mut samples = Vec::new();
    let keep_audio = loop {
        match stop.try_recv() {
            Ok(keep) => break keep,
            Err(TryRecvError::Disconnected) => break false,
            Err(TryRecvError::Empty) => {}
        }
        match input.recv_timeout(READ_INTERVAL) {
            AudioInputEvent::Chunk { samples: chunk, .. } => samples.extend(chunk),
            AudioInputEvent::Timeout => {}
            AudioInputEvent::StreamFailed(message) => {
                return Err(error(format!("microphone stream failed: {message}")));
            }
        }
    };

    drop(input);
    if keep_audio {
        AudioClip::from_pcm(samples, sample_rate, 1).map(Some)
    } else {
        Ok(None)
    }
}
