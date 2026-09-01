import { describe, expect, it } from "vitest"

import { ConnectionEpoch } from "./lifecycle"

describe("ConnectionEpoch", () => {
  it("rejects updates from an obsolete connection attempt", () => {
    const epoch = new ConnectionEpoch()
    const first = epoch.begin()
    const second = epoch.begin()

    expect(epoch.isCurrent(first)).toBe(false)
    expect(epoch.isCurrent(second)).toBe(true)

    epoch.invalidate()
    expect(epoch.isCurrent(second)).toBe(false)
  })
})
