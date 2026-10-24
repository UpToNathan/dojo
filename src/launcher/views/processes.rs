//! The Processes command: what is running, by program, and a way to stop it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dojo::launcher_core::processes::{
    self, ProcessGroup, ProcessInfo, Refusal, Sampler, SortKey, kill_process, terminate_process,
};
use dojo::launcher_core::rows::{Row, RowAction, RowIcon};
use nucleo_matcher::Matcher;

use super::{Host, Notify, Outcome, Secondary, View};

/// How often the list is sampled while the view is open.
const REFRESH: Duration = Duration::from_secs(2);
/// The wait before the second sample, which is the first to have CPU figures.
const FIRST_RESAMPLE: Duration = Duration::from_millis(400);
/// How long a second press may follow the first and still count as the confirmation.
const CONFIRM_WINDOW: Duration = Duration::from_secs(5);

type Signal = fn(&ProcessInfo) -> anyhow::Result<()>;

/// A stop or kill waiting for its confirming second press.
struct Armed {
    root_pid: u32,
    kill: bool,
    at: Instant,
}

struct Snapshot {
    groups: Vec<ProcessGroup>,
    error: Option<String>,
}

pub struct ProcessesView {
    snapshot: Arc<Mutex<Snapshot>>,
    /// Stops the sampling thread when the view is dropped.
    stop: Arc<AtomicBool>,
    sort: SortKey,
    /// The groups behind the rows last returned, by the rows' `Item` position.
    shown: Vec<ProcessGroup>,
    armed: Option<Armed>,
    matcher: Matcher,
    terminate: Signal,
    kill: Signal,
    now: fn() -> Instant,
}

impl Drop for ProcessesView {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

impl ProcessesView {
    pub fn new() -> Self {
        Self::with(Vec::new(), terminate_process, kill_process, Instant::now)
    }

    fn with(
        groups: Vec<ProcessGroup>,
        terminate: Signal,
        kill: Signal,
        now: fn() -> Instant,
    ) -> Self {
        Self {
            snapshot: Arc::new(Mutex::new(Snapshot {
                groups,
                error: None,
            })),
            stop: Arc::new(AtomicBool::new(false)),
            sort: SortKey::Cpu,
            shown: Vec::new(),
            armed: None,
            matcher: Matcher::new(nucleo_matcher::Config::DEFAULT),
            terminate,
            kill,
            now,
        }
    }

    /// Signals every process of a group, helpers first, and says what happened.
    fn signal_group(&self, group: &ProcessGroup, kill: bool) -> Outcome {
        let signal = if kill { self.kill } else { self.terminate };
        let (mut done, mut skipped) = (0, 0);
        let mut first_error = None;
        for member in group.members.iter().rev() {
            match signal(member) {
                Ok(()) => done += 1,
                Err(error) => {
                    // Dojo's own process is never signalled; a process that already left is fine.
                    let ignored = matches!(
                        error.downcast_ref::<Refusal>(),
                        Some(Refusal::OwnProcess | Refusal::Gone)
                    );
                    if ignored {
                        skipped += 1;
                    } else if first_error.is_none() {
                        first_error = Some(format!("{error:#}"));
                    }
                }
            }
        }
        let verb = if kill { "Killed" } else { "Stopped" };
        match (done, first_error) {
            (0, Some(error)) => {
                Outcome::Error(format!("Could not stop {}: {error}", group.program))
            }
            (0, None) => Outcome::Notice(format!("{} was already gone", group.program)),
            (done, error) => {
                let mut message = format!(
                    "{verb} {} process{} of {}",
                    done,
                    if done == 1 { "" } else { "es" },
                    group.program
                );
                if let Some(error) = error {
                    message.push_str(&format!(" ({error})"));
                } else if skipped > 0 {
                    message.push_str(&format!(" ({skipped} skipped)"));
                }
                Outcome::Notice(message)
            }
        }
    }

    fn arm_or_run(&mut self, row: &Row, kill: bool) -> Option<Outcome> {
        let RowAction::Item(index) = row.action else {
            return None;
        };
        let group = self.shown.get(index)?.clone();
        let now = (self.now)();
        let confirmed = self.armed.as_ref().is_some_and(|armed| {
            armed.root_pid == group.root_pid
                && armed.kill == kill
                && now.duration_since(armed.at) <= CONFIRM_WINDOW
        });
        if confirmed {
            self.armed = None;
            return Some(self.signal_group(&group, kill));
        }
        self.armed = Some(Armed {
            root_pid: group.root_pid,
            kill,
            at: now,
        });
        let count = group.members.len();
        let of = if count > 1 {
            format!(
                "{} and {} helper{}",
                group.program,
                count - 1,
                if count == 2 { "" } else { "s" }
            )
        } else {
            group.program.clone()
        };
        let (verb, key) = if kill {
            ("force-kill", "Ctrl+D")
        } else {
            ("stop", "Enter")
        };
        Some(Outcome::Notice(format!("Press {key} again to {verb} {of}")))
    }
}

impl View for ProcessesView {
    fn placeholder(&self) -> &str {
        "Search running programs…"
    }

    fn open(&mut self, notify: Notify) {
        let snapshot = self.snapshot.clone();
        let stop = self.stop.clone();
        std::thread::spawn(move || {
            let mut sampler = Sampler::new();
            // CPU is a change between two samples, so the first one reads zero: take a second
            // soon after, then settle into the normal cadence.
            let mut interval = FIRST_RESAMPLE;
            while !stop.load(Ordering::Relaxed) {
                let result = sampler.refresh();
                if let Ok(mut snapshot) = snapshot.lock() {
                    match result {
                        Ok(list) => {
                            snapshot.groups = processes::group(&list);
                            snapshot.error = None;
                        }
                        Err(error) => snapshot.error = Some(format!("{error:#}")),
                    }
                }
                notify();
                let mut waited = Duration::ZERO;
                while waited < interval && !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50));
                    waited += Duration::from_millis(50);
                }
                interval = REFRESH;
            }
        });
    }

    fn rows(&mut self, query: &str) -> Vec<Row> {
        let mut groups = self
            .snapshot
            .lock()
            .map(|snapshot| snapshot.groups.clone())
            .unwrap_or_default();
        processes::sort_groups(&mut groups, self.sort);
        let query = query.trim();
        let order = if query.is_empty() {
            (0..groups.len()).collect::<Vec<_>>()
        } else {
            processes::search(&groups, query, &mut self.matcher)
        };
        self.shown = order
            .into_iter()
            .map(|index| groups[index].clone())
            .collect();
        // A group that left the list no longer awaits confirmation.
        if let Some(armed) = &self.armed {
            if !self
                .shown
                .iter()
                .any(|group| group.root_pid == armed.root_pid)
            {
                self.armed = None;
            }
        }
        self.shown
            .iter()
            .enumerate()
            .map(|(position, group)| {
                let count = group.members.len();
                Row {
                    id: String::new(),
                    title: if count > 1 {
                        format!("{} ×{count}", group.program)
                    } else {
                        group.program.clone()
                    },
                    subtitle: format!(
                        "CPU {:.1}% · {} · pid {}",
                        group.cpu_percent,
                        human_bytes(group.rss_bytes),
                        group.root_pid
                    ),
                    icon: RowIcon::Glyph("⚙".into()),
                    action: RowAction::Item(position),
                }
            })
            .collect()
    }

    fn activate(&mut self, row: &Row, _query: &str, _host: &Host) -> Outcome {
        self.arm_or_run(row, false).unwrap_or(Outcome::Stay)
    }

    fn secondary(&mut self, key: Secondary, row: &Row, _host: &Host) -> Option<Outcome> {
        match key {
            Secondary::Delete => self.arm_or_run(row, true),
            _ => None,
        }
    }

    fn tab(&mut self) -> Option<String> {
        self.sort = match self.sort {
            SortKey::Cpu => SortKey::Memory,
            SortKey::Memory => SortKey::Cpu,
        };
        Some(match self.sort {
            SortKey::Cpu => "Sorted by CPU · Tab for memory".into(),
            SortKey::Memory => "Sorted by memory · Tab for CPU".into(),
        })
    }

    fn hint(&self) -> &str {
        "Tab sort · Ctrl+D force-kill"
    }

    fn empty_text(&self, query: &str) -> String {
        let error = self
            .snapshot
            .lock()
            .ok()
            .and_then(|snapshot| snapshot.error.clone());
        match error {
            Some(error) => format!("Could not read processes: {error}"),
            None if query.trim().is_empty() => "Reading processes…".into(),
            None => "No matching programs".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU32;

    use super::*;

    fn process(pid: u32, program: &str, cpu: f64, rss: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent_pid: 1,
            name: program.chars().take(15).collect(),
            program: program.into(),
            cmdline: program.into(),
            state: 'S',
            cpu_percent: cpu,
            rss_bytes: rss,
            threads: 1,
            start_ticks: 100,
        }
    }

    fn group(program: &str, cpu: f64, rss: u64, helpers: u32, root: u32) -> ProcessGroup {
        let mut members = vec![process(root, program, cpu, rss)];
        members.extend((1..=helpers).map(|n| process(root + n, program, 0.0, 10)));
        ProcessGroup {
            program: program.into(),
            root_pid: root,
            cmdline: program.into(),
            cpu_percent: cpu,
            rss_bytes: rss + u64::from(helpers) * 10,
            members,
        }
    }

    static SIGNALLED: AtomicU32 = AtomicU32::new(0);
    static KILLED: AtomicU32 = AtomicU32::new(0);

    fn view(groups: Vec<ProcessGroup>) -> ProcessesView {
        ProcessesView::with(
            groups,
            |_| {
                SIGNALLED.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            |_| {
                KILLED.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            Instant::now,
        )
    }

    fn sample() -> Vec<ProcessGroup> {
        vec![
            group("firefox", 12.5, 1_200_000_000, 3, 100),
            group("kitty", 0.5, 80_000_000, 0, 200),
            group("cargo", 90.0, 400_000_000, 1, 300),
        ]
    }

    #[test]
    fn programs_list_by_cpu_then_memory_with_counts_and_figures() {
        let mut view = view(sample());
        let rows = view.rows("");
        let titles = rows
            .iter()
            .map(|row| row.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(titles, ["cargo ×2", "firefox ×4", "kitty"]);
        assert_eq!(rows[0].subtitle, "CPU 90.0% · 400 MB · pid 300");
        assert_eq!(view.tab().unwrap(), "Sorted by memory · Tab for CPU");
        let titles = view
            .rows("")
            .into_iter()
            .map(|row| row.title)
            .collect::<Vec<_>>();
        assert_eq!(titles, ["firefox ×4", "cargo ×2", "kitty"]);
        assert_eq!(view.rows("kit")[0].title, "kitty");
        assert!(view.rows("zzzz").is_empty());
        assert_eq!(view.empty_text("zzzz"), "No matching programs");
    }

    #[test]
    fn stopping_needs_a_second_press_and_signals_the_whole_group() {
        let mut view = view(sample());
        let before = SIGNALLED.load(Ordering::SeqCst);
        let rows = view.rows("firefox");
        assert_eq!(
            view.activate(&rows[0], "firefox", &Host::default()),
            Outcome::Notice("Press Enter again to stop firefox and 3 helpers".into())
        );
        assert_eq!(
            SIGNALLED.load(Ordering::SeqCst),
            before,
            "nothing is signalled yet"
        );
        assert_eq!(
            view.activate(&rows[0], "firefox", &Host::default()),
            Outcome::Notice("Stopped 4 processes of firefox".into())
        );
        assert_eq!(SIGNALLED.load(Ordering::SeqCst), before + 4);
    }

    #[test]
    fn force_kill_is_a_different_confirmation_than_stop() {
        let mut view = view(sample());
        let kills = KILLED.load(Ordering::SeqCst);
        let rows = view.rows("kitty");
        assert_eq!(
            view.secondary(Secondary::Delete, &rows[0], &Host::default()),
            Some(Outcome::Notice(
                "Press Ctrl+D again to force-kill kitty".into()
            ))
        );
        // Enter after Ctrl+D is a fresh request for a different signal, not a confirmation.
        assert_eq!(
            view.activate(&rows[0], "kitty", &Host::default()),
            Outcome::Notice("Press Enter again to stop kitty".into())
        );
        assert_eq!(
            view.secondary(Secondary::Delete, &rows[0], &Host::default()),
            Some(Outcome::Notice(
                "Press Ctrl+D again to force-kill kitty".into()
            ))
        );
        assert_eq!(
            view.secondary(Secondary::Delete, &rows[0], &Host::default()),
            Some(Outcome::Notice("Killed 1 process of kitty".into()))
        );
        assert_eq!(KILLED.load(Ordering::SeqCst), kills + 1);
        assert_eq!(
            view.secondary(Secondary::Pin, &rows[0], &Host::default()),
            None
        );
    }

    #[test]
    fn a_confirmation_expires_and_does_not_carry_to_another_program() {
        let mut view = view(sample());
        let rows = view.rows("");
        view.activate(&rows[0], "", &Host::default());
        // Pressing on a different row re-arms for that row instead of signalling.
        assert!(matches!(
            view.activate(&rows[1], "", &Host::default()),
            Outcome::Notice(message) if message.starts_with("Press Enter again to stop firefox")
        ));
        // A confirmation that comes late starts over.
        view.armed.as_mut().unwrap().at = Instant::now() - CONFIRM_WINDOW - Duration::from_secs(1);
        assert!(matches!(
            view.activate(&rows[1], "", &Host::default()),
            Outcome::Notice(message) if message.starts_with("Press Enter again")
        ));
    }

    #[test]
    fn refusals_are_explained_and_own_or_departed_processes_are_skipped() {
        let mut view = ProcessesView::with(
            sample(),
            |process| {
                if process.pid == 101 {
                    return Err(Refusal::OwnProcess.into());
                }
                if process.pid == 102 {
                    return Err(Refusal::Gone.into());
                }
                Ok(())
            },
            |_| Err(Refusal::NotYours.into()),
            Instant::now,
        );
        let rows = view.rows("firefox");
        view.activate(&rows[0], "firefox", &Host::default());
        assert_eq!(
            view.activate(&rows[0], "firefox", &Host::default()),
            Outcome::Notice("Stopped 2 processes of firefox (2 skipped)".into())
        );
        let rows = view.rows("kitty");
        view.secondary(Secondary::Delete, &rows[0], &Host::default());
        assert_eq!(
            view.secondary(Secondary::Delete, &rows[0], &Host::default()),
            Some(Outcome::Error(
                "Could not stop kitty: that process belongs to another user".into()
            ))
        );
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1_500), "1.5 kB");
        assert_eq!(human_bytes(340_000_000), "340 MB");
        assert_eq!(human_bytes(1_234_000_000), "1.2 GB");
    }
}
