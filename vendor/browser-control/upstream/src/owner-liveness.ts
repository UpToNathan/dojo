import { existsSync, readFileSync } from "node:fs"

/**
 * An MCP process that owns browser leases names itself `browser-control-<pid>-<start>-<uuid>`, where
 * `start` is the process's start time from /proc. A lease whose owner process has gone can then be
 * recognised and retired, so a restarted MCP server (or a crashed one that never got to cancel) does
 * not leave a generation behind that nobody can resume or cancel.
 */
const OWNER_PATTERN = /^browser-control-(\d+)-(\d+)-/

/** Field 22 of /proc/<pid>/stat: when the process started, in clock ticks since boot. */
export function processStartTime(pid: number | "self"): string | undefined {
  try {
    const stat = readFileSync(`/proc/${pid}/stat`, "utf8")
    // The command name is in parentheses and may contain spaces; the fields after it do not.
    return stat.slice(stat.lastIndexOf(")") + 2).split(" ")[19]
  } catch {
    return undefined
  }
}

/** A new owner name for this process, unique across restarts. */
export function makeOwnerName(): string {
  return `browser-control-${process.pid}-${processStartTime("self") ?? "0"}-${crypto.randomUUID()}`
}

/**
 * Whether the process behind an owner name is gone. An owner that does not name a process (one set by
 * `DOJO_BROWSER_OWNER`, or on a system without /proc) cannot be checked, and is never called gone.
 */
export function ownerProcessIsGone(owner: string): boolean {
  const match = OWNER_PATTERN.exec(owner)
  if (!match || !existsSync("/proc/self/stat")) return false
  return processStartTime(Number(match[1])) !== match[2]
}
