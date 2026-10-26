import { describe, expect, it } from "vitest"
import { makeOwnerName, ownerProcessIsGone, processStartTime } from "../src/owner-liveness.ts"

describe("owner liveness", () => {
  it("names this process and sees it alive", () => {
    const owner = makeOwnerName()
    expect(owner).toMatch(new RegExp(`^browser-control-${process.pid}-\\d+-`))
    expect(ownerProcessIsGone(owner)).toBe(false)
  })

  it("calls an owner gone when its process does not exist or is a different one under the same pid", () => {
    expect(ownerProcessIsGone("browser-control-999999999-1-aaaa")).toBe(true)
    // Same pid as this process, but a start time that is not this process's: the pid was reused.
    const start = processStartTime("self")
    expect(start).toBeDefined()
    expect(ownerProcessIsGone(`browser-control-${process.pid}-${Number(start) + 1}-aaaa`)).toBe(true)
  })

  it("never calls an owner gone when it does not name a process", () => {
    expect(ownerProcessIsGone("agent-a")).toBe(false)
    expect(ownerProcessIsGone("browser-control-not-a-pid")).toBe(false)
  })
})
