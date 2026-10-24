//! 11 Processes: list and signal. See .scratch/launcher-features/issues/ for the ticket.
//!
//! Pure logic lives here; the launcher wires it in. Keep I/O behind small functions so the rest is testable.
//!
//! The engine reads `/proc` for the current user's processes. Parsing of `stat`, `status` and
//! `cmdline` text is pure, and the filesystem reads take the `/proc` root as a parameter, so a
//! fixture directory stands in for it in tests.
//!
//! * [`list`] samples twice, a short interval apart, and returns [`ProcessInfo`]s with CPU
//!   percentages. [`Sampler`] keeps the previous sample so a view that refreshes every second
//!   needs no sleep.
//! * [`group`] folds a program's helpers under the program (a browser's content processes under
//!   the browser) into [`ProcessGroup`]s with summed CPU and memory; [`sort_groups`] and
//!   [`search`] order and filter them.
//! * [`terminate`] and [`kill`] send SIGTERM and SIGKILL. The launcher owns the confirmation step.
//!
//! CPU percent is of one core, as in `top`: a process using two cores fully reads 200. Memory is
//! resident set size, so a group's sum counts pages its processes share more than once and can
//! exceed what the program really occupies.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Read as _;
use std::os::fd::{FromRawFd as _, OwnedFd};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32String};

/// `PF_KTHREAD` in `/proc/<pid>/stat` flags: a kernel thread.
const PF_KTHREAD: u64 = 0x0020_0000;
/// Command lines are shown, not parsed; stop reading a pathological one here.
const CMDLINE_READ_LIMIT: u64 = 16 * 1024;
const CMDLINE_DISPLAY_LIMIT: usize = 2000;
/// How long [`list`] waits between its two samples.
pub const DEFAULT_SAMPLE_INTERVAL: Duration = Duration::from_millis(250);

/// One running process of the current user.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: u32,
    /// The kernel's name for the process (`comm`, at most 15 bytes).
    pub name: String,
    /// What the process is: the basename of its executable as it was started, or `name` when the
    /// command line is empty or a rewritten title. Helpers of one program share it.
    pub program: String,
    /// The command line, arguments joined by spaces, cut at 2000 characters. Empty for zombies.
    pub cmdline: String,
    /// Kernel state letter: `R`, `S`, `D`, `Z`, `T`, ...
    pub state: char,
    /// Percent of one core over the sampling interval.
    pub cpu_percent: f64,
    pub rss_bytes: u64,
    pub threads: u32,
    /// Start time in clock ticks since boot. With the pid it identifies the process across pid
    /// reuse.
    pub start_ticks: u64,
}

/// A program and the processes started under it that share its program name.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessGroup {
    pub program: String,
    /// The topmost process of the group. Its command line stands for the group.
    pub root_pid: u32,
    pub cmdline: String,
    pub cpu_percent: f64,
    pub rss_bytes: u64,
    /// The root first, then the rest by memory, largest first.
    pub members: Vec<ProcessInfo>,
}

// ---------------------------------------------------------------------------------------------
// Parsing

/// The fields of `/proc/<pid>/stat` the engine uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatFields {
    pub pid: u32,
    pub comm: String,
    pub state: char,
    pub parent_pid: u32,
    pub flags: u64,
    /// User plus system time, in clock ticks.
    pub cpu_ticks: u64,
    pub threads: u32,
    pub start_ticks: u64,
    pub rss_pages: u64,
}

/// Parses one `/proc/<pid>/stat` line. The name may hold spaces and parentheses, so the fields
/// after it are found from its last `)`.
pub fn parse_stat(text: &str) -> Option<StatFields> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close < open {
        return None;
    }
    let pid = text[..open].trim().parse().ok()?;
    let comm = text[open + 1..close].to_owned();
    let rest = text[close + 1..].split_whitespace().collect::<Vec<_>>();
    // `rest[0]` is field 3 (state), so field N is `rest[N - 3]`.
    let field = |number: usize| rest.get(number - 3).copied();
    let utime: u64 = field(14)?.parse().ok()?;
    let stime: u64 = field(15)?.parse().ok()?;
    Some(StatFields {
        pid,
        comm,
        state: field(3)?.chars().next()?,
        parent_pid: field(4)?.parse().ok()?,
        flags: field(9)?.parse().ok()?,
        cpu_ticks: utime.saturating_add(stime),
        threads: field(20)?.parse().ok()?,
        start_ticks: field(22)?.parse().ok()?,
        rss_pages: field(24)?.parse::<i64>().ok()?.max(0) as u64,
    })
}

/// The fields of `/proc/<pid>/status` the engine uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusFields {
    pub name: String,
    /// Real user id.
    pub uid: u32,
    /// Resident set size in kilobytes; absent for kernel threads and zombies.
    pub rss_kb: Option<u64>,
}

pub fn parse_status(text: &str) -> Option<StatusFields> {
    let mut name = None;
    let mut uid = None;
    let mut rss_kb = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            "Name" => name = Some(value.to_owned()),
            "Uid" => uid = value.split_whitespace().next().and_then(|v| v.parse().ok()),
            "VmRSS" => rss_kb = value.split_whitespace().next().and_then(|v| v.parse().ok()),
            _ => {}
        }
    }
    Some(StatusFields {
        name: name?,
        uid: uid?,
        rss_kb,
    })
}

/// Splits `/proc/<pid>/cmdline` into its arguments. Invalid UTF-8 is replaced; empty trailing
/// pieces from the final NUL are dropped.
pub fn parse_cmdline(bytes: &[u8]) -> Vec<String> {
    let mut parts = bytes
        .split(|&byte| byte == 0)
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect::<Vec<_>>();
    while parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    parts
}

/// The program a process is an instance of: the basename of `argv[0]`, unless that is empty or a
/// rewritten title (`nginx: worker process`), in which case the kernel's name.
pub fn program_name(args: &[String], comm: &str) -> String {
    let Some(first) = args.first() else {
        return comm.to_owned();
    };
    if first.is_empty() || first.contains(char::is_whitespace) {
        return comm.to_owned();
    }
    let base = first.rsplit('/').next().unwrap_or(first);
    let base = base.strip_prefix('-').unwrap_or(base); // login shells run as `-bash`
    if base.is_empty() {
        comm.to_owned()
    } else {
        base.to_owned()
    }
}

fn display_cmdline(args: &[String]) -> String {
    let joined = args.join(" ");
    if joined.chars().count() > CMDLINE_DISPLAY_LIMIT {
        let mut cut = joined
            .chars()
            .take(CMDLINE_DISPLAY_LIMIT)
            .collect::<String>();
        cut.push('…');
        cut
    } else {
        joined
    }
}

// ---------------------------------------------------------------------------------------------
// Sampling

/// A process as read at one moment, before CPU percent is known.
#[derive(Clone, Debug, PartialEq)]
struct RawProcess {
    info: ProcessInfo,
    cpu_ticks: u64,
}

/// Every process of the current user at one moment.
#[derive(Clone, Debug)]
pub struct Sample {
    taken: Instant,
    processes: Vec<RawProcess>,
}

/// Machine constants that turn `/proc` numbers into units.
#[derive(Clone, Copy, Debug)]
pub struct Units {
    pub clock_ticks_per_second: u64,
    pub page_size: u64,
}

impl Units {
    pub fn system() -> Self {
        // SAFETY: sysconf only reads system configuration.
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        Self {
            clock_ticks_per_second: u64::try_from(ticks).ok().filter(|&t| t > 0).unwrap_or(100),
            page_size: u64::try_from(page).ok().filter(|&p| p > 0).unwrap_or(4096),
        }
    }
}

fn read_cmdline(path: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Ok(file) = fs::File::open(path) {
        let _ = file.take(CMDLINE_READ_LIMIT).read_to_end(&mut bytes);
    }
    bytes
}

/// Reads one process of `uid` under `proc_root`. `None` when it has gone, is unreadable, belongs
/// to someone else or is a kernel thread.
fn read_process(proc_root: &Path, pid: u32, uid: u32, units: Units) -> Option<RawProcess> {
    let directory = proc_root.join(pid.to_string());
    let stat = parse_stat(&fs::read_to_string(directory.join("stat")).ok()?)?;
    if stat.flags & PF_KTHREAD != 0 || stat.pid == 2 || stat.parent_pid == 2 {
        return None;
    }
    let status = parse_status(&fs::read_to_string(directory.join("status")).ok()?)?;
    if status.uid != uid {
        return None;
    }
    let args = parse_cmdline(&read_cmdline(&directory.join("cmdline")));
    let rss_bytes = status
        .rss_kb
        .map_or(stat.rss_pages.saturating_mul(units.page_size), |kb| {
            kb.saturating_mul(1024)
        });
    Some(RawProcess {
        info: ProcessInfo {
            pid: stat.pid,
            parent_pid: stat.parent_pid,
            program: program_name(&args, &stat.comm),
            name: stat.comm,
            cmdline: display_cmdline(&args),
            state: stat.state,
            cpu_percent: 0.0,
            rss_bytes,
            threads: stat.threads,
            start_ticks: stat.start_ticks,
        },
        cpu_ticks: stat.cpu_ticks,
    })
}

fn sample_from(proc_root: &Path, uid: u32, units: Units) -> Result<Sample> {
    let taken = Instant::now();
    let mut processes = fs::read_dir(proc_root)
        .with_context(|| format!("reading {}", proc_root.display()))?
        .filter_map(|entry| {
            let name = entry.ok()?.file_name();
            let pid = name.to_str()?.parse::<u32>().ok()?;
            read_process(proc_root, pid, uid, units)
        })
        .collect::<Vec<_>>();
    processes.sort_by_key(|process| process.info.pid);
    Ok(Sample { taken, processes })
}

/// Samples the current user's processes now.
pub fn sample() -> Result<Sample> {
    sample_from(Path::new("/proc"), current_uid(), Units::system())
}

/// Percent of one core used by `ticks` of CPU time over `elapsed`.
pub fn cpu_percent(ticks: u64, elapsed: Duration, units: Units) -> f64 {
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 || units.clock_ticks_per_second == 0 {
        return 0.0;
    }
    ticks as f64 / units.clock_ticks_per_second as f64 / seconds * 100.0
}

/// Processes in `current`, with CPU percent from their growth since `previous`. A process that
/// is new, or whose pid was reused (a different start time), reads 0.
fn with_cpu(previous: &Sample, current: &Sample, units: Units) -> Vec<ProcessInfo> {
    let before = previous
        .processes
        .iter()
        .map(|process| (process.info.pid, process))
        .collect::<HashMap<_, _>>();
    let elapsed = current.taken.saturating_duration_since(previous.taken);
    current
        .processes
        .iter()
        .map(|process| {
            let mut info = process.info.clone();
            info.cpu_percent = before
                .get(&info.pid)
                .filter(|old| old.info.start_ticks == info.start_ticks)
                .map_or(0.0, |old| {
                    cpu_percent(
                        process.cpu_ticks.saturating_sub(old.cpu_ticks),
                        elapsed,
                        units,
                    )
                });
            info
        })
        .collect()
}

/// The current user's processes with CPU percent measured over `interval`. Blocks for `interval`.
pub fn list(interval: Duration) -> Result<Vec<ProcessInfo>> {
    let units = Units::system();
    let first = sample()?;
    std::thread::sleep(interval);
    let second = sample()?;
    Ok(with_cpu(&first, &second, units))
}

/// Keeps the previous sample so each refresh measures CPU since the one before it.
#[derive(Default)]
pub struct Sampler {
    previous: Option<Sample>,
}

impl Sampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the processes now. The first call has nothing to compare with and reports 0% CPU.
    pub fn refresh(&mut self) -> Result<Vec<ProcessInfo>> {
        let current = sample()?;
        let processes = match &self.previous {
            Some(previous) => with_cpu(previous, &current, Units::system()),
            None => with_cpu(&current, &current, Units::system()),
        };
        self.previous = Some(current);
        Ok(processes)
    }
}

// ---------------------------------------------------------------------------------------------
// Grouping, sorting and search

/// Folds each process under the topmost ancestor that runs the same program, so a browser's
/// helpers join the browser. A process whose parent runs another program (a shell in a terminal)
/// starts its own group. The result is ordered by root pid; use [`sort_groups`].
pub fn group(processes: &[ProcessInfo]) -> Vec<ProcessGroup> {
    let by_pid = processes
        .iter()
        .map(|process| (process.pid, process))
        .collect::<HashMap<_, _>>();
    let root_of = |process: &ProcessInfo| {
        let mut current = process;
        // The step limit only guards against a corrupt parent cycle.
        for _ in 0..processes.len() {
            match by_pid.get(&current.parent_pid) {
                Some(parent) if parent.pid != current.pid && parent.program == current.program => {
                    current = parent;
                }
                _ => break,
            }
        }
        current.pid
    };
    let mut members: HashMap<u32, Vec<ProcessInfo>> = HashMap::new();
    for process in processes {
        members
            .entry(root_of(process))
            .or_default()
            .push(process.clone());
    }
    let mut groups = members
        .into_iter()
        .map(|(root_pid, mut members)| {
            members.sort_by(|left, right| {
                (right.pid == root_pid)
                    .cmp(&(left.pid == root_pid))
                    .then_with(|| right.rss_bytes.cmp(&left.rss_bytes))
                    .then_with(|| left.pid.cmp(&right.pid))
            });
            let root = &members[0];
            ProcessGroup {
                program: root.program.clone(),
                root_pid,
                cmdline: root.cmdline.clone(),
                cpu_percent: members.iter().map(|member| member.cpu_percent).sum(),
                rss_bytes: members.iter().map(|member| member.rss_bytes).sum(),
                members,
            }
        })
        .collect::<Vec<_>>();
    groups.sort_by_key(|group| group.root_pid);
    groups
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Cpu,
    Memory,
}

/// Largest first by the key, the other figure next, then program name and pid.
pub fn sort_groups(groups: &mut [ProcessGroup], key: SortKey) {
    groups.sort_by(|left, right| {
        let by_cpu = right.cpu_percent.total_cmp(&left.cpu_percent);
        let by_memory = right.rss_bytes.cmp(&left.rss_bytes);
        match key {
            SortKey::Cpu => by_cpu.then(by_memory),
            SortKey::Memory => by_memory.then(by_cpu),
        }
        .then_with(|| left.program.cmp(&right.program))
        .then_with(|| left.root_pid.cmp(&right.root_pid))
    });
}

/// Groups matching `query`, as indices into `groups`, best first.
///
/// An empty query returns every group in the given order. Otherwise a group matches through its
/// program name, any member's name, or any member's command line, in tiers: a pid typed in full;
/// an exact program name; a program or member name starting with the query; a fuzzy name match;
/// then every word of the query appearing in a command line. Within a tier the given order
/// stands, so sort the groups first.
pub fn search(groups: &[ProcessGroup], query: &str, matcher: &mut Matcher) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        return (0..groups.len()).collect();
    }
    let pid = query.parse::<u32>().ok();
    let pattern = Pattern::new(
        query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let atom = |kind| {
        Atom::new(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            kind,
            false,
        )
    };
    let exact = atom(AtomKind::Exact);
    let prefix = atom(AtomKind::Prefix);
    let words = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();

    let mut matched = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        let tier = (|| {
            if pid.is_some_and(|pid| group.members.iter().any(|member| member.pid == pid)) {
                return Some(0);
            }
            let program = Utf32String::from(group.program.as_str());
            if exact.score(program.slice(..), matcher).is_some() {
                return Some(1);
            }
            let names = std::iter::once(group.program.as_str())
                .chain(group.members.iter().map(|member| member.name.as_str()))
                .map(Utf32String::from)
                .collect::<Vec<_>>();
            if names
                .iter()
                .any(|name| prefix.score(name.slice(..), matcher).is_some())
            {
                return Some(2);
            }
            if names
                .iter()
                .any(|name| pattern.score(name.slice(..), matcher).is_some())
            {
                return Some(3);
            }
            group
                .members
                .iter()
                .any(|member| {
                    let lowered = member.cmdline.to_lowercase();
                    words.iter().all(|word| lowered.contains(word.as_str()))
                })
                .then_some(4)
        })();
        if let Some(tier) = tier {
            matched.push((tier, index));
        }
    }
    matched.sort();
    matched.into_iter().map(|(_, index)| index).collect()
}

// ---------------------------------------------------------------------------------------------
// Signals

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// SIGTERM: ask the process to exit.
    Terminate,
    /// SIGKILL: end the process without letting it clean up.
    Kill,
}

impl Signal {
    fn number(self) -> libc::c_int {
        match self {
            Self::Terminate => libc::SIGTERM,
            Self::Kill => libc::SIGKILL,
        }
    }
}

/// Why a process will not be signalled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Pid 0 (a process group), a pid past the kernel's range, or pid 1.
    ProtectedPid(u32),
    OwnProcess,
    KernelThread,
    NotYours,
    /// The process exited, or its pid now belongs to a different process than the one listed.
    Gone,
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProtectedPid(pid) => write!(formatter, "pid {pid} is protected"),
            Self::OwnProcess => formatter.write_str("that is the launcher's own process"),
            Self::KernelThread => formatter.write_str("kernel threads cannot be signalled"),
            Self::NotYours => formatter.write_str("that process belongs to another user"),
            Self::Gone => formatter.write_str("that process has already exited"),
        }
    }
}

impl std::error::Error for Refusal {}

/// What the safety checks need to know about a process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Target {
    uid: u32,
    flags: u64,
    parent_pid: u32,
    start_ticks: u64,
}

/// The pure part of the safety rules: pid 1 and 0, the launcher itself, kernel threads and other
/// users' processes are refused, as is a process whose start time is not the one the caller saw.
fn check_target(
    pid: u32,
    target: Option<Target>,
    own_pid: u32,
    uid: u32,
    expected_start: Option<u64>,
) -> Result<(), Refusal> {
    if pid <= 1 || pid > i32::MAX as u32 {
        return Err(Refusal::ProtectedPid(pid));
    }
    if pid == own_pid {
        return Err(Refusal::OwnProcess);
    }
    let target = target.ok_or(Refusal::Gone)?;
    if target.flags & PF_KTHREAD != 0 || pid == 2 || target.parent_pid == 2 {
        return Err(Refusal::KernelThread);
    }
    if target.uid != uid {
        return Err(Refusal::NotYours);
    }
    if expected_start.is_some_and(|start| start != target.start_ticks) {
        return Err(Refusal::Gone);
    }
    Ok(())
}

fn inspect(proc_root: &Path, pid: u32) -> Option<Target> {
    let directory = proc_root.join(pid.to_string());
    let stat = parse_stat(&fs::read_to_string(directory.join("stat")).ok()?)?;
    let status = parse_status(&fs::read_to_string(directory.join("status")).ok()?)?;
    Some(Target {
        uid: status.uid,
        flags: stat.flags,
        parent_pid: stat.parent_pid,
        start_ticks: stat.start_ticks,
    })
}

fn current_uid() -> u32 {
    // SAFETY: getuid cannot fail and has no preconditions.
    unsafe { libc::getuid() }
}

fn pidfd_open(pid: u32) -> Option<OwnedFd> {
    // SAFETY: pidfd_open takes a pid and flags and returns a new descriptor or -1.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
    // SAFETY: a non-negative return is a descriptor this call just created and nothing else owns.
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) })
}

fn deliver(pid: u32, signal: Signal, expected_start: Option<u64>, proc_root: &Path) -> Result<()> {
    // Hold the process by descriptor before inspecting it, so the signal reaches the process
    // that was inspected even if its pid is recycled in between. Kernels without pidfd fall
    // back to a plain kill, which keeps the narrow window every kill(2) has.
    let descriptor = if pid > 1 && pid <= i32::MAX as u32 {
        pidfd_open(pid)
    } else {
        None
    };
    check_target(
        pid,
        inspect(proc_root, pid),
        std::process::id(),
        current_uid(),
        expected_start,
    )?;
    let result = match &descriptor {
        Some(descriptor) => {
            use std::os::fd::AsRawFd as _;
            // SAFETY: the descriptor is a live pidfd; a null siginfo asks for a plain signal.
            unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    descriptor.as_raw_fd(),
                    signal.number(),
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                )
            }
        }
        // SAFETY: kill takes plain integers.
        None => i64::from(unsafe { libc::kill(pid as libc::pid_t, signal.number()) }),
    };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Err(Refusal::Gone.into());
    }
    Err(error).with_context(|| format!("sending {signal:?} to pid {pid}"))
}

/// Sends SIGTERM to `pid`. Refuses pid 1, the launcher's own pid, kernel threads and processes
/// of other users; the error then downcasts to [`Refusal`].
pub fn terminate(pid: u32) -> Result<()> {
    deliver(pid, Signal::Terminate, None, Path::new("/proc"))
}

/// Sends SIGKILL to `pid` under the same rules as [`terminate`].
pub fn kill(pid: u32) -> Result<()> {
    deliver(pid, Signal::Kill, None, Path::new("/proc"))
}

/// [`terminate`] for a process from a listing: refused as gone if the pid now belongs to a
/// process that started later.
pub fn terminate_process(process: &ProcessInfo) -> Result<()> {
    deliver(
        process.pid,
        Signal::Terminate,
        Some(process.start_ticks),
        Path::new("/proc"),
    )
}

/// [`kill`] for a process from a listing, checked as [`terminate_process`] is.
pub fn kill_process(process: &ProcessInfo) -> Result<()> {
    deliver(
        process.pid,
        Signal::Kill,
        Some(process.start_ticks),
        Path::new("/proc"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt as _;

    const UNITS: Units = Units {
        clock_ticks_per_second: 100,
        page_size: 4096,
    };

    // A real line from a running system, with a name that holds a space and parentheses.
    const STAT: &str = "4242 (Web Content (x)) S 4100 4100 4100 0 -1 4194560 12345 0 0 0 \
        150 50 0 0 20 0 7 0 987654 1234567890 25600 18446744073709551615 1 1 0 0 0 0 0 0 \
        16781312 0 0 0 17 3 0 0 0 0 0 0 0 0 0 0 0 0 0";

    #[test]
    fn stat_parses_names_with_spaces_and_parentheses() {
        let stat = parse_stat(STAT).unwrap();
        assert_eq!(stat.pid, 4242);
        assert_eq!(stat.comm, "Web Content (x)");
        assert_eq!(stat.state, 'S');
        assert_eq!(stat.parent_pid, 4100);
        assert_eq!(stat.flags, 4_194_560);
        assert_eq!(stat.cpu_ticks, 200);
        assert_eq!(stat.threads, 7);
        assert_eq!(stat.start_ticks, 987_654);
        assert_eq!(stat.rss_pages, 25_600);
    }

    #[test]
    fn stat_rejects_truncated_or_malformed_lines() {
        assert_eq!(parse_stat(""), None);
        assert_eq!(parse_stat("12 (x) S 1 2 3"), None);
        assert_eq!(parse_stat("not a stat line"), None);
        assert_eq!(
            parse_stat("x (a) S 1 1 1 0 -1 0 0 0 0 0 1 1 0 0 20 0 1 0 5 0 0"),
            None
        );
    }

    #[test]
    fn stat_clamps_a_negative_rss() {
        let line = STAT.replace(" 25600 ", " -1 ");
        assert_eq!(parse_stat(&line).unwrap().rss_pages, 0);
    }

    const STATUS: &str = "Name:\tWeb Content\nUmask:\t0022\nState:\tS (sleeping)\nTgid:\t4242\n\
        Uid:\t1000\t1000\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\nVmRSS:\t  102400 kB\n";

    #[test]
    fn status_yields_name_uid_and_rss() {
        let status = parse_status(STATUS).unwrap();
        assert_eq!(status.name, "Web Content");
        assert_eq!(status.uid, 1000);
        assert_eq!(status.rss_kb, Some(102_400));
    }

    #[test]
    fn status_without_vmrss_is_still_valid_and_without_uid_is_not() {
        let status = parse_status("Name:\tkthreadd\nUid:\t0\t0\t0\t0\n").unwrap();
        assert_eq!(status.rss_kb, None);
        assert_eq!(parse_status("Name:\tx\n"), None);
        assert_eq!(parse_status("Uid:\t1\n"), None);
    }

    #[test]
    fn cmdline_splits_on_nul_and_tolerates_odd_bytes() {
        assert_eq!(
            parse_cmdline(b"/usr/bin/firefox\0-contentproc\0--flag=a b\0"),
            ["/usr/bin/firefox", "-contentproc", "--flag=a b"]
        );
        assert!(parse_cmdline(b"").is_empty());
        assert_eq!(parse_cmdline(b"a\0\0b\0"), ["a", "", "b"]);
        assert_eq!(parse_cmdline(b"bad\xff\0"), ["bad\u{fffd}"]);
    }

    #[test]
    fn program_names_come_from_argv0_unless_it_is_a_title() {
        let args = |line: &str| line.split('\0').map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(
            program_name(&args("/usr/lib/firefox/firefox\0-x"), "Web Content"),
            "firefox"
        );
        assert_eq!(program_name(&args("-bash"), "bash"), "bash");
        assert_eq!(
            program_name(&args("nginx: worker process"), "nginx"),
            "nginx"
        );
        assert_eq!(program_name(&[], "kworker"), "kworker");
        assert_eq!(program_name(&args("/"), "x"), "x");
    }

    #[test]
    fn long_command_lines_are_cut() {
        let args = vec!["x".repeat(5000)];
        let shown = display_cmdline(&args);
        assert_eq!(shown.chars().count(), CMDLINE_DISPLAY_LIMIT + 1);
        assert!(shown.ends_with('…'));
    }

    #[test]
    fn cpu_percent_is_ticks_over_elapsed_time() {
        let second = Duration::from_secs(1);
        assert!((cpu_percent(50, second, UNITS) - 50.0).abs() < 1e-9);
        assert!((cpu_percent(100, Duration::from_millis(500), UNITS) - 200.0).abs() < 1e-9);
        assert_eq!(cpu_percent(10, Duration::ZERO, UNITS), 0.0);
    }

    /// Writes one process into a fake `/proc`.
    fn write_process(
        root: &Path,
        pid: u32,
        comm: &str,
        parent: u32,
        uid: u32,
        flags: u64,
        cpu: (u64, u64),
        rss_kb: Option<u64>,
        cmdline: &[&str],
    ) {
        let directory = root.join(pid.to_string());
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("stat"),
            format!(
                "{pid} ({comm}) S {parent} {pid} {pid} 0 -1 {flags} 0 0 0 0 {} {} 0 0 20 0 3 0 {} 0 {} 0 0\n",
                cpu.0,
                cpu.1,
                pid * 10,
                rss_kb.unwrap_or(0) / 4,
            ),
        )
        .unwrap();
        let rss = rss_kb.map_or(String::new(), |kb| format!("VmRSS:\t{kb} kB\n"));
        fs::write(
            directory.join("status"),
            format!("Name:\t{comm}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n{rss}"),
        )
        .unwrap();
        let mut bytes = Vec::new();
        for argument in cmdline {
            bytes.extend_from_slice(argument.as_bytes());
            bytes.push(0);
        }
        fs::write(directory.join("cmdline"), bytes).unwrap();
    }

    fn fixture_proc() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        write_process(
            root,
            1,
            "systemd",
            0,
            0,
            0,
            (1, 1),
            Some(10_000),
            &["/sbin/init"],
        );
        write_process(root, 2, "kthreadd", 0, 0, PF_KTHREAD, (0, 0), None, &[]);
        write_process(root, 3, "kworker", 2, 1000, PF_KTHREAD, (0, 0), None, &[]);
        write_process(
            root,
            100,
            "bash",
            1,
            1000,
            0,
            (5, 5),
            Some(4_000),
            &["-bash"],
        );
        write_process(
            root,
            200,
            "firefox",
            100,
            1000,
            0,
            (100, 20),
            Some(400_000),
            &["/usr/lib/firefox/firefox", "--new-window"],
        );
        write_process(
            root,
            201,
            "Web Content",
            200,
            1000,
            0,
            (50, 10),
            Some(300_000),
            &["/usr/lib/firefox/firefox", "-contentproc", "1"],
        );
        write_process(
            root,
            202,
            "Isolated Web Co",
            200,
            1000,
            0,
            (10, 0),
            Some(100_000),
            &["/usr/lib/firefox/firefox", "-contentproc", "2"],
        );
        write_process(
            root,
            300,
            "other",
            1,
            1001,
            0,
            (9, 9),
            Some(5_000),
            &["other"],
        );
        // Not a process directory.
        fs::create_dir_all(root.join("sys")).unwrap();
        fs::write(root.join("uptime"), "1 1\n").unwrap();
        directory
    }

    #[test]
    fn sampling_a_fixture_keeps_only_this_users_real_processes() {
        let proc = fixture_proc();
        let sample = sample_from(proc.path(), 1000, UNITS).unwrap();
        let pids = sample
            .processes
            .iter()
            .map(|process| process.info.pid)
            .collect::<Vec<_>>();
        assert_eq!(pids, [100, 200, 201, 202]);
        let firefox = &sample.processes[1];
        assert_eq!(firefox.info.rss_bytes, 400_000 * 1024);
        assert_eq!(firefox.info.program, "firefox");
        assert_eq!(
            firefox.info.cmdline,
            "/usr/lib/firefox/firefox --new-window"
        );
        assert_eq!(firefox.cpu_ticks, 120);
        assert_eq!(sample.processes[0].info.program, "bash");
    }

    #[test]
    fn a_vanished_process_directory_is_skipped_not_fatal() {
        let proc = fixture_proc();
        fs::remove_file(proc.path().join("201/stat")).unwrap();
        let sample = sample_from(proc.path(), 1000, UNITS).unwrap();
        assert_eq!(sample.processes.len(), 3);
    }

    #[test]
    fn cpu_comes_from_tick_growth_between_two_samples() {
        let proc = fixture_proc();
        let first = sample_from(proc.path(), 1000, UNITS).unwrap();
        // Firefox burns 50 ticks over one second; the web content process was replaced by a new
        // process under the same pid; the isolated process is untouched.
        write_process(
            proc.path(),
            200,
            "firefox",
            100,
            1000,
            0,
            (130, 40),
            Some(400_000),
            &["/usr/lib/firefox/firefox", "--new-window"],
        );
        let mut second = sample_from(proc.path(), 1000, UNITS).unwrap();
        second.taken = first.taken + Duration::from_secs(1);
        let replaced = second
            .processes
            .iter_mut()
            .find(|p| p.info.pid == 201)
            .unwrap();
        replaced.info.start_ticks += 1;
        replaced.cpu_ticks += 500;

        let infos = with_cpu(&first, &second, UNITS);
        let cpu = |pid: u32| infos.iter().find(|p| p.pid == pid).unwrap().cpu_percent;
        assert!((cpu(200) - 50.0).abs() < 1e-9);
        assert_eq!(
            cpu(201),
            0.0,
            "a reused pid must not inherit the old process's counters"
        );
        assert_eq!(cpu(202), 0.0);
        assert_eq!(cpu(100), 0.0);
    }

    #[test]
    fn a_new_process_reads_zero_cpu() {
        let proc = fixture_proc();
        let empty = Sample {
            taken: Instant::now(),
            processes: Vec::new(),
        };
        let later = sample_from(proc.path(), 1000, UNITS).unwrap();
        assert!(
            with_cpu(&empty, &later, UNITS)
                .iter()
                .all(|p| p.cpu_percent == 0.0)
        );
    }

    fn fixture_groups() -> Vec<ProcessGroup> {
        let proc = fixture_proc();
        let first = sample_from(proc.path(), 1000, UNITS).unwrap();
        let mut second = first.clone();
        second.taken = first.taken + Duration::from_secs(1);
        for process in &mut second.processes {
            process.cpu_ticks += match process.info.pid {
                200 => 10,
                201 => 40,
                202 => 5,
                100 => 1,
                _ => 0,
            };
        }
        group(&with_cpu(&first, &second, UNITS))
    }

    #[test]
    fn helpers_group_under_their_program_with_summed_figures() {
        let groups = fixture_groups();
        assert_eq!(
            groups.len(),
            2,
            "bash stays apart from firefox: it is another program"
        );
        let firefox = groups.iter().find(|g| g.program == "firefox").unwrap();
        assert_eq!(firefox.root_pid, 200);
        assert_eq!(
            firefox.members.iter().map(|m| m.pid).collect::<Vec<_>>(),
            [200, 201, 202]
        );
        assert_eq!(firefox.rss_bytes, 800_000 * 1024);
        assert!((firefox.cpu_percent - 55.0).abs() < 1e-6);
        assert_eq!(firefox.cmdline, "/usr/lib/firefox/firefox --new-window");
        let bash = groups.iter().find(|g| g.program == "bash").unwrap();
        assert_eq!(bash.members.len(), 1);
    }

    #[test]
    fn unrelated_instances_and_orphans_form_separate_groups() {
        let info = |pid, parent, program: &str| ProcessInfo {
            pid,
            parent_pid: parent,
            name: program.to_owned(),
            program: program.to_owned(),
            cmdline: String::new(),
            state: 'S',
            cpu_percent: 1.0,
            rss_bytes: 10,
            threads: 1,
            start_ticks: 1,
        };
        // Two terminals, one with a helper; a helper whose parent is gone; a pid cycle.
        let groups = group(&[
            info(10, 1, "kitty"),
            info(11, 10, "kitty"),
            info(20, 1, "kitty"),
            info(30, 999, "kitty"),
            info(40, 41, "loop"),
            info(41, 40, "loop"),
        ]);
        let roots = groups
            .iter()
            .map(|g| (g.root_pid, g.members.len()))
            .collect::<Vec<_>>();
        assert!(roots.contains(&(10, 2)));
        assert!(roots.contains(&(20, 1)));
        assert!(roots.contains(&(30, 1)));
        let total: usize = groups.iter().map(|g| g.members.len()).sum();
        assert_eq!(
            total, 6,
            "every process lands in exactly one group, even in a cycle"
        );
    }

    #[test]
    fn groups_sort_by_cpu_or_memory() {
        let mut groups = fixture_groups();
        sort_groups(&mut groups, SortKey::Memory);
        assert_eq!(groups[0].program, "firefox");
        sort_groups(&mut groups, SortKey::Cpu);
        assert_eq!(groups[0].program, "firefox");
        // Make bash the CPU hog and check the keys really differ.
        groups
            .iter_mut()
            .find(|g| g.program == "bash")
            .unwrap()
            .cpu_percent = 900.0;
        sort_groups(&mut groups, SortKey::Cpu);
        assert_eq!(groups[0].program, "bash");
        sort_groups(&mut groups, SortKey::Memory);
        assert_eq!(groups[0].program, "firefox");
    }

    fn matcher() -> Matcher {
        Matcher::new(nucleo_matcher::Config::DEFAULT)
    }

    fn found(groups: &[ProcessGroup], query: &str) -> Vec<String> {
        search(groups, query, &mut matcher())
            .into_iter()
            .map(|index| groups[index].program.clone())
            .collect()
    }

    #[test]
    fn search_matches_names_helpers_command_lines_and_pids() {
        let groups = fixture_groups();
        assert_eq!(found(&groups, "").len(), 2);
        assert_eq!(found(&groups, "firefox"), ["firefox"]);
        assert_eq!(found(&groups, "fire"), ["firefox"]);
        assert_eq!(
            found(&groups, "isolated"),
            ["firefox"],
            "a helper's own name finds its group"
        );
        assert_eq!(
            found(&groups, "contentproc"),
            ["firefox"],
            "command lines are searched"
        );
        assert_eq!(found(&groups, "new-window firefox"), ["firefox"]);
        assert_eq!(
            found(&groups, "202"),
            ["firefox"],
            "a pid finds the group holding it"
        );
        assert_eq!(found(&groups, "bash"), ["bash"]);
        assert!(found(&groups, "zzzz").is_empty());
    }

    #[test]
    fn exact_program_names_rank_above_prefix_and_command_line_matches() {
        let info = |pid, program: &str, cmdline: &str| ProcessInfo {
            pid,
            parent_pid: 1,
            name: program.to_owned(),
            program: program.to_owned(),
            cmdline: cmdline.to_owned(),
            state: 'S',
            cpu_percent: 0.0,
            rss_bytes: 0,
            threads: 1,
            start_ticks: 1,
        };
        let groups = group(&[
            info(10, "mentions", "mentions vim in the args vim"),
            info(11, "vimtutor", "vimtutor"),
            info(12, "vim", "vim"),
        ]);
        assert_eq!(found(&groups, "vim"), ["vim", "vimtutor", "mentions"]);
    }

    #[test]
    fn refusals_cover_pid_one_zero_self_kernel_threads_and_strangers() {
        let user = Target {
            uid: 1000,
            flags: 0,
            parent_pid: 100,
            start_ticks: 5,
        };
        let check = |pid, target, expected| check_target(pid, target, 4000, 1000, expected);
        assert_eq!(check(1, Some(user), None), Err(Refusal::ProtectedPid(1)));
        assert_eq!(check(0, Some(user), None), Err(Refusal::ProtectedPid(0)));
        assert_eq!(
            check(u32::MAX, Some(user), None),
            Err(Refusal::ProtectedPid(u32::MAX))
        );
        assert_eq!(check(4000, Some(user), None), Err(Refusal::OwnProcess));
        assert_eq!(check(5, None, None), Err(Refusal::Gone));
        let kthread = Target {
            flags: PF_KTHREAD,
            ..user
        };
        assert_eq!(check(5, Some(kthread), None), Err(Refusal::KernelThread));
        assert_eq!(check(2, Some(user), None), Err(Refusal::KernelThread));
        let child_of_kthreadd = Target {
            parent_pid: 2,
            ..user
        };
        assert_eq!(
            check(5, Some(child_of_kthreadd), None),
            Err(Refusal::KernelThread)
        );
        let stranger = Target { uid: 0, ..user };
        assert_eq!(check(5, Some(stranger), None), Err(Refusal::NotYours));
        assert_eq!(check(5, Some(user), Some(6)), Err(Refusal::Gone));
        assert_eq!(check(5, Some(user), Some(5)), Ok(()));
        assert_eq!(check(5, Some(user), None), Ok(()));
    }

    #[test]
    fn inspect_reads_the_safety_fields_from_a_fixture() {
        let proc = fixture_proc();
        let target = inspect(proc.path(), 200).unwrap();
        assert_eq!(
            (target.uid, target.parent_pid, target.start_ticks),
            (1000, 100, 2000)
        );
        assert!(inspect(proc.path(), 12345).is_none());
        assert!(inspect(proc.path(), 3).unwrap().flags & PF_KTHREAD != 0);
    }

    #[test]
    fn signalling_refuses_protected_targets_before_sending_anything() {
        for pid in [0, 1, std::process::id()] {
            let error = terminate(pid).unwrap_err();
            assert!(
                error.downcast_ref::<Refusal>().is_some(),
                "pid {pid}: {error}"
            );
            assert!(kill(pid).is_err());
        }
        // A pid no process holds.
        let error = terminate(i32::MAX as u32).unwrap_err();
        assert_eq!(error.downcast_ref::<Refusal>(), Some(&Refusal::Gone));
    }

    #[test]
    fn terminate_and_kill_reach_a_child_of_this_process() {
        for (use_kill, expected) in [(false, libc::SIGTERM), (true, libc::SIGKILL)] {
            let mut child = std::process::Command::new("sleep")
                .arg("60")
                .spawn()
                .unwrap();
            let pid = child.id();
            // The child must be visible with the figures the listing shows before it is signalled.
            let snapshot = sample().unwrap();
            let listed = snapshot
                .processes
                .iter()
                .find(|process| process.info.pid == pid)
                .expect("the child is listed")
                .info
                .clone();
            assert_eq!(listed.program, "sleep");
            assert_eq!(listed.parent_pid, std::process::id());

            // A stale listing entry (wrong start time) must not be signalled.
            let mut stale = listed.clone();
            stale.start_ticks += 1;
            assert!(terminate_process(&stale).is_err());
            assert!(child.try_wait().unwrap().is_none());

            if use_kill {
                kill_process(&listed).unwrap();
            } else {
                terminate_process(&listed).unwrap();
            }
            let status = child.wait().unwrap();
            assert_eq!(status.signal(), Some(expected));
        }
    }

    #[test]
    fn plain_terminate_and_kill_by_pid_work_on_a_child() {
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        terminate(child.id()).unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGTERM));
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        kill(child.id()).unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
    }

    #[test]
    fn live_listing_includes_this_test_process_with_plausible_figures() {
        let processes = list(Duration::from_millis(100)).unwrap();
        let me = processes
            .iter()
            .find(|process| process.pid == std::process::id())
            .expect("this process is listed");
        assert!(me.rss_bytes > 0);
        assert!(me.threads >= 1);
        assert!(me.cpu_percent >= 0.0);
        assert!(!me.cmdline.is_empty());
        assert!(
            processes
                .iter()
                .all(|process| process.pid != 1 || process.rss_bytes > 0)
        );
        let groups = group(&processes);
        let total: usize = groups.iter().map(|group| group.members.len()).sum();
        assert_eq!(total, processes.len());
        assert!(
            groups
                .iter()
                .any(|group| group.members.iter().any(|m| m.pid == me.pid))
        );
    }

    #[test]
    fn sampler_reports_zero_then_measured_cpu() {
        let mut sampler = Sampler::new();
        let first = sampler.refresh().unwrap();
        assert!(first.iter().all(|process| process.cpu_percent == 0.0));
        // Burn a little CPU so the second refresh sees growth.
        let started = Instant::now();
        let mut value = 0u64;
        while started.elapsed() < Duration::from_millis(120) {
            value = value.wrapping_add(std::hint::black_box(1));
        }
        std::hint::black_box(value);
        let second = sampler.refresh().unwrap();
        assert!(
            second
                .iter()
                .any(|process| process.pid == std::process::id())
        );
    }
}
