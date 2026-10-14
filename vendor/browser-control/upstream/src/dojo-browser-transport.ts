import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process"
import { createInterface } from "node:readline"
import { isExtensionEvent, isExtensionResponse, parseJsonObject, type ExtensionCommand, type ExtensionEvent, type ExtensionResponse, type JsonObject } from "./protocol.ts"

export type DojoLease = Readonly<{ lease: string; generation: number }>

export const dojoBrokerMaxLineBytes = 1_048_576

export function dojoLeaseId(owner: string, sessionId: string): string {
  return JSON.stringify([owner, sessionId])
}

type BrokerResult = {
  readonly op: "result"
  readonly id: number
  readonly status: "admitted" | "refused" | "unknown" | "retired" | "bound"
  readonly reason?: string
}

type Pending = {
  readonly resolve: (result: BrokerResult) => void
  readonly reject: (error: Error) => void
  readonly timeout: ReturnType<typeof setTimeout>
}

export class DojoBrowserTransport {
  private readonly child: ChildProcessWithoutNullStreams
  private readonly pending = new Map<number, Pending>()
  private nextId = 1
  private connectedState = false
  private closed = false

  constructor(
    executable: string,
    private readonly onExtensionMessage: (message: ExtensionEvent | ExtensionResponse) => void,
    private readonly onConnectionChange: (connected: boolean) => void,
  ) {
    this.child = spawn(executable, ["--stdio"], { stdio: ["pipe", "pipe", "pipe"] })
    this.child.stderr.pipe(process.stderr)
    this.child.stdin.on("error", (cause: Error) => {
      this.connectedState = false
      this.onConnectionChange(false)
      this.failAll(new Error("Write to native Browser Control broker", { cause }))
    })
    const lines = createInterface({ input: this.child.stdout })
    lines.on("line", (line) => this.handleLine(line))
    this.child.once("error", (cause) => this.failAll(new Error("Start native Browser Control broker", { cause })))
    this.child.once("exit", (code, signal) => {
      this.connectedState = false
      this.onConnectionChange(false)
      this.failAll(new Error(`Native Browser Control broker exited (${code ?? signal ?? "unknown"})`))
    })
  }

  get connected(): boolean {
    return this.connectedState && !this.closed
  }

  send(options: { readonly lease: DojoLease; readonly command: ExtensionCommand }): Promise<BrokerResult> {
    return this.request({
      op: "send",
      lease: options.lease.lease,
      generation: options.lease.generation,
      command: {
        ...options.command,
        dojoLease: options.lease.lease,
        dojoGeneration: options.lease.generation,
      },
    })
  }

  retire(options: { readonly lease: DojoLease }): Promise<BrokerResult> {
    return this.request({ op: "retire", lease: options.lease.lease, generation: options.lease.generation })
  }

  bind(options: { readonly lease: DojoLease }): Promise<BrokerResult> {
    return this.request({ op: "bind", lease: options.lease.lease, generation: options.lease.generation })
  }

  close(): void {
    this.closed = true
    this.child.kill("SIGTERM")
    this.failAll(new Error("Native Browser Control transport closed"))
  }

  private request(request: object): Promise<BrokerResult> {
    if (this.closed || this.child.stdin.destroyed) {
      return Promise.reject(new Error("Native Browser Control broker is unavailable"))
    }
    const id = this.nextId
    const body = `${JSON.stringify({ ...request, id })}\n`
    const byteLength = Buffer.byteLength(body, "utf8")
    if (byteLength > dojoBrokerMaxLineBytes) {
      return Promise.resolve({
        op: "result",
        id: 0,
        status: "refused",
        reason: `Native browser request exceeds the ${dojoBrokerMaxLineBytes}-byte line limit (${byteLength} bytes)`,
      })
    }
    this.nextId += 1
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`Native Browser Control broker response timed out for request ${id}`))
      }, 10_000)
      this.pending.set(id, { resolve, reject, timeout })
      this.child.stdin.write(body, (cause) => {
        if (cause) {
          const pending = this.pending.get(id)
          this.pending.delete(id)
          if (pending) clearTimeout(pending.timeout)
          const error = new Error("Write to native Browser Control broker", { cause })
          this.failAll(error)
          reject(error)
        }
      })
    })
  }

  private handleLine(line: string): void {
    let value: JsonObject
    try {
      value = parseJsonObject(line)
    } catch {
      this.failAll(new Error("Native Browser Control broker emitted invalid JSON"))
      return
    }
    if (value.op === "result") {
      const id = value.id
      const status = value.status
      const reason = value.reason
      if (typeof id !== "number" || !Number.isSafeInteger(id) || typeof status !== "string") return
      if (status !== "admitted" && status !== "refused" && status !== "unknown" && status !== "retired" && status !== "bound") return
      const result: BrokerResult = { op: "result", id, status, ...(typeof reason === "string" ? { reason } : {}) }
      const pending = this.pending.get(id)
      if (!pending) return
      this.pending.delete(id)
      clearTimeout(pending.timeout)
      pending.resolve(result)
      return
    }
    if (value.op === "extension") {
      const message = value.message
      if (message && typeof message === "object" && !Array.isArray(message)) {
        if (isExtensionEvent(message)) this.onExtensionMessage(message)
        else if (isExtensionResponse(message)) this.onExtensionMessage(message)
      }
      return
    }
    if (value.op === "connected" || value.op === "disconnected") {
      const connected = value.op === "connected"
      this.connectedState = connected
      this.onConnectionChange(connected)
    }
  }

  private failAll(error: Error): void {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timeout)
      pending.reject(error)
    }
    this.pending.clear()
  }
}
