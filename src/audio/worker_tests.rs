use super::*;

#[test]
fn batch_wait_is_event_driven_not_polled() {
    // Structural regression guard: the only batch waits must be channel waits.
    // The bounded pre-read try_recv drain in run_worker is deliberately separate.
    let source = include_str!("../audio.rs");
    let batch = source
        .split("fn next_batch(")
        .nth(1)
        .unwrap()
        .split("// This is the actual subprocess seam")
        .next()
        .unwrap();
    assert!(batch.contains("receiver.recv().ok()"));
    assert!(
        batch.contains("receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))")
    );
    assert!(!batch.contains("try_recv"));
    assert!(!batch.contains("sleep"));
    assert!(!batch.contains("from_millis(1)"));
}

#[test]
fn idle_worker_blocks_until_command_then_flushes_without_another_event() {
    let fake = Arc::new(Mutex::new(Fake::default()));
    let (sender, changes, worker) = start(fake.clone());
    assert_eq!(
        changes.recv_timeout(Duration::from_millis(50)),
        Err(mpsc::RecvTimeoutError::Timeout)
    );
    assert!(fake.lock().unwrap().calls.is_empty());
    sender.send(AudioCommand::Set(81)).unwrap();
    assert_eq!(receive(&changes).percent, 81);
    assert_eq!(
        changes.recv_timeout(Duration::from_millis(50)),
        Err(mpsc::RecvTimeoutError::Timeout)
    );
    sender.send(AudioCommand::Stop).unwrap();
    worker.join().unwrap();
}

#[test]
fn disconnect_flushes_latest_set_and_terminates() {
    let fake = Arc::new(Mutex::new(Fake::default()));
    let (sender, receiver) = mpsc::channel();
    sender.send(AudioCommand::Set(12)).unwrap();
    sender.send(AudioCommand::Set(92)).unwrap();
    drop(sender);
    let mut states = Vec::new();
    run_worker(
        receiver,
        fake.clone(),
        |v| {
            states.push(v.unwrap());
            true
        },
        &AudioLifecycle::default(),
    );
    assert_eq!(
        states,
        vec![Volume {
            percent: 92,
            muted: false
        }]
    );
    assert_eq!(fake.lock().unwrap().calls.len(), 2);
}

#[test]
fn owner_drop_wakes_worker_with_subscription_sender_still_alive_and_delivers_tail() {
    let fake = Arc::new(Mutex::new(Fake::default()));
    let lifecycle = Arc::new(AudioLifecycle::default());
    let (commands, changes, worker) = start_with_lifecycle(fake, lifecycle.clone());
    let events = commands.clone();
    let owner = AudioWorker {
        commands,
        lifecycle: lifecycle.clone(),
    };
    owner.commands.send(AudioCommand::Set(14)).unwrap();
    owner.commands.send(AudioCommand::Set(89)).unwrap();
    drop(owner);
    assert!(lifecycle.is_stopped());
    assert_eq!(receive(&changes).percent, 89);
    assert_eq!(
        changes.recv_timeout(Duration::from_secs(2)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
    worker.join().unwrap();
    assert!(events.send(AudioCommand::Refresh).is_err());
    lifecycle.stop(); // Shutdown is idempotent across owner/control exits.
}

#[test]
fn owner_drop_flushes_barrier_tail_after_update_receiver_closes() {
    let fake = Arc::new(Mutex::new(Fake::default()));
    let (commands, receiver) = mpsc::channel();
    let events = commands.clone(); // Subscription sender outlives the owner.
    let lifecycle = Arc::new(AudioLifecycle::default());
    let owner = AudioWorker {
        commands,
        lifecycle: lifecycle.clone(),
    };
    let (updates, changes) = async_channel::unbounded();
    let (entered, first_publication) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let backend = fake.clone();
    let worker_lifecycle = lifecycle.clone();
    let worker = std::thread::spawn(move || {
        let mut first = true;
        run_worker(
            receiver,
            backend,
            |state| {
                if first {
                    first = false;
                    entered.send(state).unwrap();
                    resume.recv_timeout(Duration::from_secs(2)).unwrap();
                }
                updates.send_blocking(state).is_ok()
            },
            &worker_lifecycle,
        );
        done.send(()).unwrap();
    });
    // The mute barrier prevents these sets from collapsing into one batch.
    for command in [
        AudioCommand::Set(20),
        AudioCommand::ToggleMute,
        AudioCommand::Set(89),
    ] {
        owner.commands.send(command).unwrap();
    }
    assert_eq!(
        receive(&first_publication),
        Volume {
            percent: 20,
            muted: false
        }
    );
    // Reproduce AudioState's field destruction: drop the worker, then the task
    // owning the updates receiver, before allowing the first publish to fail.
    drop(owner);
    drop(changes);
    assert!(lifecycle.is_stopped());
    release.send(()).unwrap();
    finished.recv_timeout(Duration::from_secs(2)).unwrap();
    worker.join().unwrap();
    assert!(events.send(AudioCommand::Refresh).is_err());
    let fake = fake.lock().unwrap();
    assert_eq!(fake.level, 89.);
    assert!(
        !fake.muted,
        "the final set must unmute after the mute barrier"
    );
    let writes: Vec<_> = fake
        .calls
        .iter()
        .filter(|c| c[0] != "get-volume")
        .map(|c| (c[0].as_str(), c.last().unwrap().as_str()))
        .collect();
    assert_eq!(
        writes,
        vec![
            ("set-volume", "20%"),
            ("set-mute", "toggle"),
            ("set-volume", "89%"),
            ("set-mute", "0"),
        ]
    );
}

#[test]
fn worker_exit_disconnects_retained_senders_when_publication_stops() {
    let fake = Arc::new(Mutex::new(Fake::default()));
    let (sender, receiver) = mpsc::channel();
    sender.send(AudioCommand::ToggleMute).unwrap();
    sender.send(AudioCommand::Set(89)).unwrap();
    let lifecycle = AudioLifecycle::default();
    run_worker(receiver, fake.clone(), |_| false, &lifecycle);
    assert!(!lifecycle.is_stopped());
    assert!(sender.send(AudioCommand::Refresh).is_err());
    let fake = fake.lock().unwrap();
    assert_eq!(
        fake.level, 0.,
        "live-owner publication failure must exit, not drain"
    );
    assert!(fake.muted);
}

#[derive(Default)]
struct Fake {
    level: f32,
    muted: bool,
    calls: Vec<Vec<String>>,
    fail_write: bool,
    events_on_write: Option<mpsc::Sender<AudioCommand>>,
}

impl AudioBackend for Arc<Mutex<Fake>> {
    fn command(&mut self, args: &[&str]) -> anyhow::Result<String> {
        let mut fake = self.lock().unwrap();
        fake.calls
            .push(args.iter().map(|s| s.to_string()).collect());
        match args[0] {
            "get-volume" => {
                return Ok(format!(
                    "Volume: {}{}",
                    fake.level / 100.,
                    if fake.muted { " [MUTED]" } else { "" }
                ));
            }
            "set-volume" => {
                if fake.fail_write {
                    bail!("injected write failure");
                }
                let value = args.last().unwrap();
                let relative = value.ends_with('+') || value.ends_with('-');
                let number: f32 = value
                    .trim_end_matches(['+', '-'])
                    .trim_end_matches('%')
                    .parse()?;
                fake.level = if relative {
                    fake.level
                        + if value.ends_with('+') {
                            number
                        } else {
                            -number
                        }
                } else {
                    number
                }
                .clamp(0., 100.);
            }
            "set-mute" => {
                fake.muted = match args[2] {
                    "toggle" => !fake.muted,
                    "0" => false,
                    _ => panic!("unexpected mute argument"),
                }
            }
            _ => panic!("unexpected command"),
        }
        if let Some(events) = &fake.events_on_write {
            for _ in 0..10 {
                events.send(AudioCommand::Refresh).unwrap();
            }
        }
        Ok(String::new())
    }
}

fn start(
    fake: Arc<Mutex<Fake>>,
) -> (
    mpsc::Sender<AudioCommand>,
    std::sync::mpsc::Receiver<Option<Volume>>,
    std::thread::JoinHandle<()>,
) {
    start_with_lifecycle(fake, Arc::new(AudioLifecycle::default()))
}

fn start_with_lifecycle(
    fake: Arc<Mutex<Fake>>,
    lifecycle: Arc<AudioLifecycle>,
) -> (
    mpsc::Sender<AudioCommand>,
    std::sync::mpsc::Receiver<Option<Volume>>,
    std::thread::JoinHandle<()>,
) {
    let (sender, receiver) = mpsc::channel();
    let (updates, changes) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        run_worker(receiver, fake, |v| updates.send(v).is_ok(), &lifecycle)
    });
    (sender, changes, thread)
}

fn receive(changes: &std::sync::mpsc::Receiver<Option<Volume>>) -> Volume {
    changes
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap()
}

#[test]
fn paced_drag_has_bounded_writes_and_delivers_tail_without_release_or_close() {
    let fake = Arc::new(Mutex::new(Fake::default()));
    let (sender, changes, worker) = start(fake.clone());
    // Keep input alive across several windows, as actual motion events do.
    for percent in 1..=80 {
        sender.send(AudioCommand::Set(percent)).unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut deliveries = 0;
    loop {
        let state = receive(&changes);
        deliveries += 1;
        if state.percent == 80 {
            break;
        }
    }
    assert!(
        deliveries > 1,
        "must flush during a sustained drag, not debounce forever"
    );
    // Timing may split windows differently, but should be far below 80 writes.
    assert!(deliveries < 40, "unexpectedly many batches: {deliveries}");
    let fake = fake.lock().unwrap();
    assert_eq!(
        fake.calls.iter().filter(|c| c[0] == "set-volume").count(),
        deliveries
    );
    assert_eq!(
        fake.calls.iter().filter(|c| c[0] == "get-volume").count(),
        deliveries
    );
    assert!(!fake.calls.iter().any(|c| c[0] == "set-mute"));
    drop(fake);
    sender.send(AudioCommand::Stop).unwrap();
    worker.join().unwrap();
}

#[test]
fn queued_sets_scroll_and_mute_keep_barrier_order_and_clamping() {
    let fake = Arc::new(Mutex::new(Fake::default()));
    let (sender, receiver) = mpsc::channel();
    for command in [
        AudioCommand::Set(20),
        AudioCommand::Set(99),
        AudioCommand::Adjust(3.),
        AudioCommand::Adjust(2.),
        AudioCommand::Adjust(-5.),
        AudioCommand::ToggleMute,
        AudioCommand::Set(30),
        AudioCommand::Set(40),
        AudioCommand::ToggleMute,
        AudioCommand::Stop,
    ] {
        sender.send(command).unwrap();
    }
    let mut states = Vec::new();
    run_worker(
        receiver,
        fake.clone(),
        |state| {
            states.push(state.unwrap());
            true
        },
        &AudioLifecycle::default(),
    );
    assert_eq!(
        states,
        vec![
            Volume {
                percent: 99,
                muted: false
            },
            Volume {
                percent: 95,
                muted: false
            },
            Volume {
                percent: 95,
                muted: true
            },
            Volume {
                percent: 40,
                muted: false
            },
            Volume {
                percent: 40,
                muted: true
            },
        ]
    );
    let fake = fake.lock().unwrap();
    let writes: Vec<_> = fake
        .calls
        .iter()
        .filter(|c| c[0] != "get-volume")
        .map(|c| (c[0].as_str(), c.last().unwrap().as_str()))
        .collect();
    assert_eq!(
        writes,
        vec![
            ("set-volume", "99%"),
            ("set-volume", "95%"),
            ("set-mute", "toggle"),
            ("set-volume", "40%"),
            ("set-mute", "0"),
            ("set-mute", "toggle")
        ]
    );
}

#[test]
fn refresh_bursts_coalesce_but_later_external_default_changes_are_read() {
    let fake = Arc::new(Mutex::new(Fake {
        level: 80.,
        ..Fake::default()
    }));
    let (sender, changes, worker) = start(fake.clone());
    sender.send(AudioCommand::Set(50)).unwrap();
    assert_eq!(
        receive(&changes),
        Volume {
            percent: 50,
            muted: false
        }
    );
    // Subscription events cannot identify their author: treat a later event as
    // live external/default-sink state, even immediately after an own write.
    {
        let mut f = fake.lock().unwrap();
        f.level = 17.;
        f.muted = true;
    }
    for _ in 0..200 {
        sender.send(AudioCommand::Refresh).unwrap();
    }
    assert_eq!(
        receive(&changes),
        Volume {
            percent: 17,
            muted: true
        }
    );
    sender.send(AudioCommand::Adjust(2.)).unwrap();
    assert_eq!(
        receive(&changes),
        Volume {
            percent: 19,
            muted: false
        }
    );
    let f = fake.lock().unwrap();
    assert_eq!(f.calls.iter().filter(|c| c[0] == "get-volume").count(), 4);
    assert_eq!(f.calls.iter().filter(|c| c[0] == "set-mute").count(), 1);
    drop(f);
    sender.send(AudioCommand::Stop).unwrap();
    worker.join().unwrap();
}

#[test]
fn scroll_burst_uses_one_write_and_keeps_fractional_adjustments() {
    let fake = Arc::new(Mutex::new(Fake::default()));
    let (sender, receiver) = mpsc::channel();
    for _ in 0..200 {
        sender.send(AudioCommand::Adjust(0.1)).unwrap();
    }
    sender.send(AudioCommand::Stop).unwrap();
    let mut states = Vec::new();
    run_worker(
        receiver,
        fake.clone(),
        |state| {
            states.push(state.unwrap());
            true
        },
        &AudioLifecycle::default(),
    );
    assert_eq!(
        states,
        vec![Volume {
            percent: 20,
            muted: false
        }]
    );
    let f = fake.lock().unwrap();
    assert_eq!(f.calls.iter().filter(|c| c[0] == "set-volume").count(), 1);
    assert_eq!(f.calls.iter().filter(|c| c[0] == "get-volume").count(), 2);
}

#[test]
fn amplified_external_volume_applies_each_adjustment_clamp() {
    let fake = Arc::new(Mutex::new(Fake {
        level: 150.,
        ..Fake::default()
    }));
    let (sender, receiver) = mpsc::channel();
    for command in [
        AudioCommand::Adjust(-10.),
        AudioCommand::Adjust(-10.),
        AudioCommand::Stop,
    ] {
        sender.send(command).unwrap();
    }
    let mut state = None;
    run_worker(
        receiver,
        fake,
        |v| {
            state = v;
            true
        },
        &AudioLifecycle::default(),
    );
    // First adjustment clamps 140 to 100; second reaches 90, not 100.
    assert_eq!(
        state,
        Some(Volume {
            percent: 90,
            muted: false
        })
    );
}

#[test]
fn subscription_events_during_write_are_covered_by_post_write_read() {
    let fake = Arc::new(Mutex::new(Fake::default()));
    let (sender, changes, worker) = start(fake.clone());
    fake.lock().unwrap().events_on_write = Some(sender.clone());
    sender.send(AudioCommand::Set(73)).unwrap();
    assert_eq!(
        receive(&changes),
        Volume {
            percent: 73,
            muted: false
        }
    );
    // Ten events emitted synchronously by the fake write must not cause an
    // additional read. A later external event must still be honored.
    {
        let mut f = fake.lock().unwrap();
        assert_eq!(f.calls.iter().filter(|c| c[0] == "get-volume").count(), 1);
        f.level = 31.;
        f.muted = true;
    }
    sender.send(AudioCommand::Refresh).unwrap();
    assert_eq!(
        receive(&changes),
        Volume {
            percent: 31,
            muted: true
        }
    );
    assert_eq!(
        fake.lock()
            .unwrap()
            .calls
            .iter()
            .filter(|c| c[0] == "get-volume")
            .count(),
        2
    );
    sender.send(AudioCommand::Stop).unwrap();
    worker.join().unwrap();
    assert!(changes.try_recv().is_err());
}

#[test]
fn standalone_mute_is_not_held_for_batching() {
    let (sender, receiver) = mpsc::channel();
    sender.send(AudioCommand::ToggleMute).unwrap();
    // Keeping the sender open would block a batching receive. The command is
    // returned immediately and does not consume or require a following command.
    assert_eq!(
        next_batch(&receiver, &mut None),
        Some(AudioCommand::ToggleMute)
    );
    assert_eq!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty));
}

#[test]
fn failed_write_reads_live_state_and_worker_keeps_running() {
    let fake = Arc::new(Mutex::new(Fake {
        level: 62.,
        muted: true,
        fail_write: true,
        ..Fake::default()
    }));
    let (sender, changes, worker) = start(fake.clone());
    sender.send(AudioCommand::Set(10)).unwrap();
    assert_eq!(
        receive(&changes),
        Volume {
            percent: 62,
            muted: true
        }
    );
    sender.send(AudioCommand::ToggleMute).unwrap();
    assert_eq!(
        receive(&changes),
        Volume {
            percent: 62,
            muted: false
        }
    );
    assert!(
        !fake
            .lock()
            .unwrap()
            .calls
            .iter()
            .any(|c| c.last().map(String::as_str) == Some("0"))
    );
    sender.send(AudioCommand::Stop).unwrap();
    worker.join().unwrap();
}
