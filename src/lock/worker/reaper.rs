//! Killing a worker and everything it started. pam_exec children call setsid(),
//! so a process-group kill misses them; the tree is snapshotted from /proc first.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Process {
    pub pid: i32,
    pub parent: i32,
    pub session: i32,
    /// Clock ticks since boot, so a reused pid is not mistaken for the original.
    pub start: u64,
}

/// One /proc/<pid>/stat line. `None` for zombies and dead entries, which hold nothing.
pub(crate) fn parse_stat(line: &str) -> Option<Process> {
    let (head, rest) = line.rsplit_once(')')?;
    let pid = head.split_once(" (")?.0.trim().parse().ok()?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // After the command: state(0) ppid(1) pgrp(2) session(3) ... starttime(19).
    if matches!(*fields.first()?, "Z" | "X" | "x") {
        return None;
    }
    Some(Process {
        pid,
        parent: fields.get(1)?.parse().ok()?,
        session: fields.get(3)?.parse().ok()?,
        start: fields.get(19)?.parse().ok()?,
    })
}

pub(crate) fn read_table() -> Vec<Process> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter_map(|entry| std::fs::read_to_string(entry.path().join("stat")).ok())
        .filter_map(|line| parse_stat(&line))
        .collect()
}

/// The root, every descendant by parent link, and every member of the root's
/// session. A session id is only ever the pid of its leader, and the kernel does
/// not reuse a pid while a session carries it, so the session test cannot catch
/// a stranger.
pub(crate) fn tree(table: &[Process], root: i32) -> Vec<Process> {
    let mut members: BTreeSet<i32> = table
        .iter()
        .filter(|process| process.pid == root || process.session == root)
        .map(|process| process.pid)
        .collect();
    loop {
        let before = members.len();
        for process in table {
            if members.contains(&process.parent) {
                members.insert(process.pid);
            }
        }
        if members.len() == before {
            break;
        }
    }
    table
        .iter()
        .filter(|process| members.contains(&process.pid))
        .copied()
        .collect()
}

pub(crate) fn alive(victims: &[Process]) -> Vec<Process> {
    let table = read_table();
    victims
        .iter()
        .filter(|victim| table.contains(victim))
        .copied()
        .collect()
}

/// SIGTERM, then SIGKILL, each with a bounded wait. `reap` collects the direct
/// child so it does not linger as a zombie. Returns what is still alive.
pub(crate) fn kill(victims: &[Process], mut reap: impl FnMut()) -> Vec<Process> {
    for signal in [libc::SIGTERM, libc::SIGKILL] {
        let living = alive(victims);
        if living.is_empty() {
            return living;
        }
        for process in &living {
            unsafe { libc::kill(process.pid, signal) };
        }
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            reap();
            if alive(victims).is_empty() {
                return Vec::new();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    reap();
    alive(victims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: i32, parent: i32, session: i32) -> Process {
        Process {
            pid,
            parent,
            session,
            start: pid as u64 * 10,
        }
    }

    fn pids(processes: &[Process]) -> Vec<i32> {
        processes.iter().map(|process| process.pid).collect()
    }

    #[test]
    fn stat_lines_parse_including_awkward_command_names() {
        let line = "4242 (pam ex) (x)) S 4200 4242 4242 0 -1 4194560 100 0 0 0 0 0 0 0 20 0 1 0 987654 1000 200 18446744073709551615";
        assert_eq!(
            parse_stat(line),
            Some(Process {
                pid: 4242,
                parent: 4200,
                session: 4242,
                start: 987654
            })
        );
        assert_eq!(
            parse_stat("4243 (sleep) Z 4242 4242 4242 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 5 0 0 0"),
            None
        );
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn tree_follows_setsid_escapes_and_session_members() {
        let table = [
            process(1, 0, 1),
            process(900, 1, 900),      // the locker
            process(1000, 900, 1000),  // the worker, a session leader
            process(1001, 1000, 1001), // pam_exec child after setsid()
            process(1002, 1001, 1001), // its sleep
            process(1003, 1, 1000),    // orphan still in the worker's session
            process(1004, 1003, 1004), // orphan's own setsid child
            process(2000, 900, 2000),  // the other worker
            process(2001, 2000, 2001),
        ];
        assert_eq!(
            pids(&tree(&table, 1000)),
            vec![1000, 1001, 1002, 1003, 1004]
        );
        assert_eq!(pids(&tree(&table, 2000)), vec![2000, 2001]);
    }

    #[test]
    fn tree_of_a_dead_root_still_finds_its_session() {
        let table = [
            process(1, 0, 1),
            process(1003, 1, 1000),
            process(1004, 1003, 1003),
        ];
        assert_eq!(pids(&tree(&table, 1000)), vec![1003, 1004]);
        assert_eq!(pids(&tree(&table, 5000)), Vec::<i32>::new());
    }

    #[test]
    fn tree_never_takes_the_parents_session() {
        // A root that is not a session leader must not drag in its session.
        let table = [
            process(900, 1, 900),
            process(901, 900, 900),
            process(1000, 900, 900),
        ];
        assert_eq!(pids(&tree(&table, 1000)), vec![1000]);
    }

    #[test]
    fn real_table_contains_this_process() {
        let me = std::process::id() as i32;
        let table = read_table();
        let found = table
            .iter()
            .find(|process| process.pid == me)
            .expect("self in /proc");
        assert_eq!(found.parent, unsafe { libc::getppid() });
        assert_eq!(found.session, unsafe { libc::getsid(0) });
    }
}
