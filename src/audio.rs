use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use gpui::{App, BorrowAppContext, Global};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    pub percent: u8,
    pub muted: bool,
}

fn parse_volume_value(text: &str) -> anyhow::Result<(f32, bool)> {
    let mut words = text.split_whitespace();
    if words.next() != Some("Volume:") {
        bail!("missing volume prefix");
    }
    let value: f32 = words.next().context("missing volume")?.parse()?;
    if !value.is_finite() || value < 0. {
        bail!("invalid volume");
    }
    let muted = match words.next() {
        None => false,
        Some("[MUTED]") => true,
        Some(_) => bail!("invalid mute state"),
    };
    if words.next().is_some() {
        bail!("unexpected volume fields");
    }
    Ok((value, muted))
}

fn parse_volume(text: &str) -> anyhow::Result<Volume> {
    let (value, muted) = parse_volume_value(text)?;
    Ok(Volume {
        percent: (value * 100.).round().min(100.) as u8,
        muted,
    })
}

fn wpctl(arguments: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("wpctl")
        .args(arguments)
        .env("LC_ALL", "C")
        .output()?;
    if !output.status.success() {
        bail!("wpctl: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8(output.stdout)?)
}

#[derive(Debug, PartialEq)]
enum AudioCommand {
    Refresh,
    Set(u8),
    Adjust(f32),
    AdjustBatch(Vec<f32>),
    ToggleMute,
    Stop,
}

// A fixed (not trailing-edge) window guarantees delivery even while dragging.
// Barriers end the window early; mute itself is never batched.
const BATCH_WINDOW: Duration = Duration::from_millis(25);

fn next_batch(
    receiver: &mpsc::Receiver<AudioCommand>,
    pending: &mut Option<AudioCommand>,
) -> Option<AudioCommand> {
    let mut command = pending.take().or_else(|| receiver.recv().ok())?;
    if matches!(command, AudioCommand::ToggleMute | AudioCommand::Stop) {
        return Some(command);
    }
    if let AudioCommand::Adjust(change) = command {
        command = AudioCommand::AdjustBatch(vec![change]);
    }
    let deadline = Instant::now() + BATCH_WINDOW;
    loop {
        if Instant::now() >= deadline {
            break;
        }
        // Wait for an event or the original deadline, never a polling tick.
        match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(next) => match (&mut command, &next) {
                (AudioCommand::Set(_) | AudioCommand::AdjustBatch(_), AudioCommand::Refresh) => {}
                (AudioCommand::Set(_), AudioCommand::Set(_))
                | (AudioCommand::Refresh, AudioCommand::Refresh)
                | (AudioCommand::Refresh, AudioCommand::Set(_)) => {
                    command = next;
                }
                (AudioCommand::Refresh, AudioCommand::Adjust(change)) => {
                    command = AudioCommand::AdjustBatch(vec![*change]);
                }
                (AudioCommand::AdjustBatch(changes), AudioCommand::Adjust(change)) => {
                    changes.push(*change);
                }
                _ => {
                    *pending = Some(next);
                    break;
                }
            },
            Err(mpsc::RecvTimeoutError::Disconnected | mpsc::RecvTimeoutError::Timeout) => break,
        }
    }
    Some(command)
}

// This is the actual subprocess seam, shared by production and worker tests.
trait AudioBackend {
    fn command(&mut self, arguments: &[&str]) -> anyhow::Result<String>;

    fn read(&mut self) -> anyhow::Result<Volume> {
        parse_volume(&self.command(&["get-volume", "@DEFAULT_AUDIO_SINK@"])?)
    }
}

struct Wpctl;
impl AudioBackend for Wpctl {
    fn command(&mut self, arguments: &[&str]) -> anyhow::Result<String> {
        wpctl(arguments)
    }
}

fn execute(
    backend: &mut impl AudioBackend,
    command: AudioCommand,
    before_read: impl FnOnce(),
) -> anyhow::Result<Volume> {
    let adjusting = matches!(
        command,
        AudioCommand::Set(_) | AudioCommand::Adjust(_) | AudioCommand::AdjustBatch(_)
    );
    match command {
        AudioCommand::Refresh => {}
        AudioCommand::Set(percent) => {
            backend.command(&[
                "set-volume",
                "--limit",
                "1.0",
                "@DEFAULT_AUDIO_SINK@",
                &format!("{percent}%"),
            ])?;
        }
        AudioCommand::Adjust(change) => {
            backend.command(&[
                "set-volume",
                "--limit",
                "1.0",
                "@DEFAULT_AUDIO_SINK@",
                &format!("{}%{}", change.abs(), if change >= 0. { "+" } else { "-" }),
            ])?;
        }
        AudioCommand::AdjustBatch(changes) => {
            if changes.len() == 1 {
                return execute(backend, AudioCommand::Adjust(changes[0]), before_read);
            }
            // Read the live wpctl value, not the UI's rounded/capped percentage
            // or a cached default. Apply each clamp in order, including reversals
            // and an externally amplified (>100%) initial level. This subprocess
            // read-modify-write is not atomic with concurrent external changes.
            let text = backend.command(&["get-volume", "@DEFAULT_AUDIO_SINK@"])?;
            let (value, _) = parse_volume_value(&text)?;
            let percent = changes.into_iter().fold(value * 100., |level, change| {
                (level + change).clamp(0., 100.)
            });
            backend.command(&[
                "set-volume",
                "--limit",
                "1.0",
                "@DEFAULT_AUDIO_SINK@",
                &format!("{percent}%"),
            ])?;
        }
        AudioCommand::ToggleMute => {
            backend.command(&["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])?;
        }
        AudioCommand::Stop => unreachable!("stop is handled by the worker"),
    }
    before_read();
    // Never reuse a previous command's default-sink/mute state. Read the live
    // default after writing: normally this is the only read and no mute write.
    let state = backend.read()?;
    if adjusting && state.muted {
        backend.command(&["set-mute", "@DEFAULT_AUDIO_SINK@", "0"])?;
        backend.read()
    } else {
        Ok(state)
    }
}

fn run_worker(
    receiver: mpsc::Receiver<AudioCommand>,
    mut backend: impl AudioBackend,
    mut publish: impl FnMut(Option<Volume>) -> bool,
    lifecycle: &AudioLifecycle,
) {
    let mut pending = None;
    let mut publishing = true;
    while let Some(command) = next_batch(&receiver, &mut pending) {
        if matches!(command, AudioCommand::Stop) {
            break;
        }
        let state = match execute(&mut backend, command, || {
            // Events already queued before this read are covered by the read,
            // regardless of their author. Never discard events after it, and
            // never cross a user-command barrier. Bound the drain under floods.
            if pending.is_none() {
                for _ in 0..256 {
                    match receiver.try_recv() {
                        Ok(AudioCommand::Refresh) => {}
                        Ok(next) => {
                            pending = Some(next);
                            break;
                        }
                        Err(_) => break,
                    }
                }
            }
        }) {
            Ok(state) => Some(state),
            Err(error) => {
                eprintln!("Dojo audio command failed: {error:#}");
                // A failed write may still have changed the backend.
                backend.read().ok()
            }
        };
        if publishing && !publish(state) {
            if !lifecycle.is_stopped() {
                // A live owner losing its receiver is not explicit shutdown:
                // exit without waiting on senders retained by the subscription.
                break;
            }
            // Owner drop stops the lifecycle before dropping the updates task.
            // Its Stop may still be in flight: retain blocking queue waits and
            // execute accepted commands through that barrier without publishing.
            publishing = false;
        }
    }
}

#[derive(Default)]
struct AudioLifecycle {
    stopped: AtomicBool,
    subscription: Mutex<Option<Child>>,
}

impl AudioLifecycle {
    fn stop(&self) {
        // Serialize with subscription installation so a racing spawn cannot
        // leave a live child blocked on stdout after the control worker exits.
        let mut subscription = self.subscription.lock().unwrap();
        self.stopped.store(true, Ordering::Release);
        if let Some(mut child) = subscription.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}

struct AudioWorker {
    commands: mpsc::Sender<AudioCommand>,
    lifecycle: Arc<AudioLifecycle>,
}

impl Drop for AudioWorker {
    fn drop(&mut self) {
        self.lifecycle.stop();
        // Wake an idle recv or an active recv_timeout. Stop is a barrier, so
        // accepted commands before it (including the final set) still flush.
        let _ = self.commands.send(AudioCommand::Stop);
    }
}

pub struct AudioState {
    pub volume: Option<Volume>,
    worker: AudioWorker,
    _updates: gpui::Task<()>,
}
impl Global for AudioState {}
impl AudioState {
    pub fn set_volume(&self, percent: u8) {
        let _ = self
            .worker
            .commands
            .send(AudioCommand::Set(percent.min(100)));
    }
    pub fn adjust_volume(&self, percent: f32) {
        if !percent.is_finite() {
            return;
        }
        let _ = self
            .worker
            .commands
            .send(AudioCommand::Adjust(percent.clamp(-100., 100.)));
    }
    pub fn toggle_mute(&self) {
        let _ = self.worker.commands.send(AudioCommand::ToggleMute);
    }
}

pub fn initialize(cx: &mut App) {
    let (commands, receiver) = mpsc::channel();
    let (updates, changes) = async_channel::unbounded();
    let lifecycle = Arc::new(AudioLifecycle::default());
    let lifecycle_for_events = lifecycle.clone();
    let events = commands.clone();
    std::thread::Builder::new()
        .name("dojo-audio-events".into())
        .spawn(move || {
            loop {
                if lifecycle_for_events.is_stopped() {
                    break;
                }
                let child = Command::new("pactl")
                    .arg("subscribe")
                    .env("LC_ALL", "C")
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn();
                if let Ok(mut child) = child {
                    let stdout = child.stdout.take().unwrap();
                    {
                        let mut subscription = lifecycle_for_events.subscription.lock().unwrap();
                        if lifecycle_for_events.is_stopped() {
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        *subscription = Some(child);
                    }
                    let _ = events.send(AudioCommand::Refresh);
                    for line in BufReader::new(stdout).lines() {
                        let Ok(line) = line else {
                            break;
                        };
                        if (line.contains(" on sink #") || line.contains(" on server #"))
                            && events.send(AudioCommand::Refresh).is_err()
                        {
                            break;
                        }
                    }
                    if let Some(mut child) =
                        lifecycle_for_events.subscription.lock().unwrap().take()
                    {
                        // A failed channel send/read can end this loop while
                        // pactl is still alive. Never wait on it without killing
                        // it, especially while holding the lifecycle lock.
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                }
                if lifecycle_for_events.is_stopped() {
                    break;
                }
                std::thread::sleep(Duration::from_secs(3));
            }
        })
        .expect("start audio event worker");
    let lifecycle_for_control = lifecycle.clone();
    std::thread::Builder::new()
        .name("dojo-audio-control".into())
        .spawn(move || {
            run_worker(
                receiver,
                Wpctl,
                |state| updates.send_blocking(state).is_ok(),
                &lifecycle_for_control,
            );
            lifecycle_for_control.stop();
        })
        .expect("start audio control worker");
    let task = cx.spawn(async move |cx| {
        while let Ok(volume) = changes.recv().await {
            let _ = cx.update(|cx| {
                if cx.global::<AudioState>().volume != volume {
                    cx.update_global::<AudioState, _>(|audio, _| audio.volume = volume);
                }
            });
        }
    });
    cx.set_global(AudioState {
        volume: None,
        worker: AudioWorker {
            commands,
            lifecycle,
        },
        _updates: task,
    });
    let _ = cx
        .global::<AudioState>()
        .worker
        .commands
        .send(AudioCommand::Refresh);
}

pub fn drag_volume(start: u8, distance: f32) -> u8 {
    (f32::from(start) + distance * 1.25).round().clamp(0., 100.) as u8
}

pub struct Wave {
    from: [f32; 8],
    target: [f32; 8],
    started: Instant,
}
impl Default for Wave {
    fn default() -> Self {
        Self {
            from: [2., 2., 2., 2., 2., 2., 2., 36.],
            target: [2., 2., 2., 2., 2., 2., 2., 36.],
            started: Instant::now(),
        }
    }
}
impl Wave {
    pub fn sample(&self, now: Instant) -> [f32; 8] {
        let elapsed = now.duration_since(self.started).as_secs_f32();
        std::array::from_fn(|index| {
            let progress = (elapsed / (0.16 + index as f32 * 0.01)).clamp(0., 1.);
            let eased = 1. - (1. - progress).powi(3);
            self.from[index] + (self.target[index] - self.from[index]) * eased
        })
    }
    pub fn update(
        &mut self,
        volume: Option<Volume>,
        expanded: bool,
        now: Instant,
    ) -> ([f32; 8], bool) {
        let level = volume
            .filter(|v| !v.muted)
            .map_or(0., |v| f32::from(v.percent) / 100.);
        let mut target = [0.2, 0.45, 0.75, 1., 0.75, 0.45, 0.2, 0.];
        for height in &mut target[..7] {
            *height = 2. + *height * level * 9.;
        }
        target[7] = if expanded { 80. } else { 36. };
        if target != self.target {
            self.from = self.sample(now);
            self.target = target;
            self.started = now;
        }
        (
            self.sample(now),
            now.duration_since(self.started) < Duration::from_millis(235),
        )
    }
}

#[cfg(test)]
#[path = "audio/worker_tests.rs"]
mod worker_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_wireplumber_volume_and_mute() {
        assert_eq!(
            parse_volume("Volume: 0.80\n").unwrap(),
            Volume {
                percent: 80,
                muted: false
            }
        );
        assert_eq!(
            parse_volume("Volume: 0.37 [MUTED]\n").unwrap(),
            Volume {
                percent: 37,
                muted: true
            }
        );
        assert_eq!(parse_volume("Volume: 1.50").unwrap().percent, 100);
        for invalid in [
            "",
            "Volume: NaN",
            "Volume: inf",
            "Volume: -1",
            "Volume: 0.5 other",
            "Volume: 0.5 [MUTED] extra",
        ] {
            assert!(parse_volume(invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn drag_is_relative_and_bounded() {
        assert_eq!(drag_volume(40, 16.), 60);
        assert_eq!(drag_volume(40, -16.), 20);
        assert_eq!(drag_volume(90, 80.), 100);
        assert_eq!(drag_volume(10, -80.), 0);
    }
    #[test]
    fn wave_settles_and_mute_flattens_without_losing_volume() {
        let mut wave = Wave::default();
        let now = Instant::now();
        wave.update(
            Some(Volume {
                percent: 100,
                muted: false,
            }),
            true,
            now,
        );
        let (heights, moving) = wave.update(
            Some(Volume {
                percent: 100,
                muted: false,
            }),
            true,
            now + Duration::from_secs(1),
        );
        for (actual, expected) in heights
            .into_iter()
            .zip([3.8, 6.05, 8.75, 11., 8.75, 6.05, 3.8, 80.])
        {
            assert!((actual - expected).abs() < 0.00001);
        }
        assert!(!moving);
        wave.update(
            Some(Volume {
                percent: 100,
                muted: true,
            }),
            false,
            now + Duration::from_secs(1),
        );
        assert_eq!(
            wave.sample(now + Duration::from_secs(2)),
            [2., 2., 2., 2., 2., 2., 2., 36.]
        );
    }
}
