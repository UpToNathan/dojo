import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import { afterEach, describe, expect, it } from "vitest"
import { DojoBrowserTransport, dojoBrokerMaxLineBytes } from "../src/dojo-browser-transport.ts"

const temporaryDirectories: string[] = []

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) fs.rmSync(directory, { recursive: true, force: true })
})

describe("DojoBrowserTransport", () => {
  it("refuses oversized lines before allocating a broker request ID", async () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), "browser-control-native-line-"))
    temporaryDirectories.push(directory)
    const recordPath = path.join(directory, "requests.jsonl")
    const executable = path.join(directory, "broker")
    fs.writeFileSync(executable, [
      "#!/usr/bin/env node",
      "const fs = require('node:fs')",
      "const readline = require('node:readline')",
      `const recordPath = ${JSON.stringify(recordPath)}`,
      "readline.createInterface({ input: process.stdin }).on('line', (line) => {",
      "  const request = JSON.parse(line)",
      "  fs.appendFileSync(recordPath, line + '\\n')",
      "  process.stdout.write(JSON.stringify({ op: 'result', id: request.id, status: 'admitted' }) + '\\n')",
      "})",
    ].join("\n"))
    fs.chmodSync(executable, 0o755)
    const transport = new DojoBrowserTransport(executable, () => {}, () => {})
    try {
      const oversized = await transport.send({
        lease: { lease: "owner/session", generation: 1 },
        command: {
          id: 1,
          method: "debugger.sendCommand",
          params: {
            tabId: 7,
            method: "Runtime.evaluate",
            params: { expression: "x".repeat(dojoBrokerMaxLineBytes) },
          },
        },
      })
      expect(oversized).toMatchObject({ op: "result", id: 0, status: "refused" })

      const admitted = await transport.bind({ lease: { lease: "small", generation: 1 } })
      expect(admitted).toMatchObject({ op: "result", id: 1, status: "admitted" })

      const lines = fs.readFileSync(recordPath, "utf8").trim().split("\n")
      expect(lines).toHaveLength(1)
      expect(JSON.parse(lines[0] ?? "{}")).toMatchObject({ op: "bind", id: 1, lease: "small" })
    } finally {
      transport.close()
    }
  })

  it("settles pending requests when the broker exits during a write", async () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), "browser-control-native-exit-"))
    temporaryDirectories.push(directory)
    const executable = path.join(directory, "broker")
    fs.writeFileSync(executable, [
      "#!/usr/bin/env node",
      "const readline = require('node:readline')",
      "readline.createInterface({ input: process.stdin }).once('line', () => process.exit(0))",
    ].join("\n"))
    fs.chmodSync(executable, 0o755)
    const transport = new DojoBrowserTransport(executable, () => {}, () => {})
    try {
      await expect(transport.bind({ lease: { lease: "owner/session", generation: 1 } })).rejects.toThrow("Native Browser Control broker exited")
    } finally {
      transport.close()
    }
  })
})
