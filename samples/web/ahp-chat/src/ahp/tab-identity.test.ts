import { describe, expect, it } from "vitest"

import { TabIdentity, type ClaimChannel } from "./tab-identity"

class ClaimHub {
  readonly channels = new Set<TestChannel>()

  open(): TestChannel {
    const channel = new TestChannel(this)
    this.channels.add(channel)
    return channel
  }
}

class TestChannel implements ClaimChannel {
  onmessage: ((event: MessageEvent<{ type: "probe" | "contend" | "claimed"; candidate: string; nonce: string }>) => void) | null = null
  private readonly hub: ClaimHub

  constructor(hub: ClaimHub) {
    this.hub = hub
  }

  postMessage(message: { type: "probe" | "contend" | "claimed"; candidate: string; nonce: string }): void {
    for (const channel of this.hub.channels) {
      if (channel !== this) channel.onmessage?.({ data: message } as MessageEvent<typeof message>)
    }
  }

  close(): void {
    this.hub.channels.delete(this)
  }
}

function identity(hub: ClaimHub, candidate: () => string, rotateCandidate: () => string, nonce: string): TabIdentity {
  return new TabIdentity({ candidate, rotateCandidate, nonce, pageId: () => `page-${nonce}`, waitMs: 1, createChannel: () => hub.open() })
}

describe("tab identity claims", () => {
  it("rotates a cloned candidate when its live opener responds", async () => {
    const hub = new ClaimHub()
    const opener = identity(hub, () => "copied", () => "opener-new", "a")
    const child = identity(hub, () => "copied", () => "child-new", "b")

    await expect(opener.uniqueTabClientId()).resolves.toBe("copied")
    await expect(child.uniqueTabClientId()).resolves.toBe("child-new")
    expect(hub.channels).toHaveLength(2)
    opener.dispose()
    child.dispose()
    expect(hub.channels).toHaveLength(0)
  })

  it("keeps exactly one candidate for simultaneous claims", async () => {
    const hub = new ClaimHub()
    const lower = identity(hub, () => "copied", () => "lower-new", "a")
    const higher = identity(hub, () => "copied", () => "higher-new", "b")

    await expect(Promise.all([lower.uniqueTabClientId(), higher.uniqueTabClientId()]))
      .resolves.toEqual(["copied", "higher-new"])
    lower.dispose()
    higher.dispose()
  })

  it("keeps a reload candidate when no live peer remains", async () => {
    const hub = new ClaimHub()
    const original = identity(hub, () => "restored", () => "rotated", "a")
    await original.uniqueTabClientId()
    original.dispose()
    const reloaded = identity(hub, () => "restored", () => "rotated", "b")

    await expect(reloaded.uniqueTabClientId()).resolves.toBe("restored")
    reloaded.dispose()
  })

  it("uses the page ID without BroadcastChannel support", async () => {
    const fallback = new TabIdentity({
      candidate: () => "shared",
      rotateCandidate: () => "rotated",
      pageId: () => "page-only",
      createChannel: () => undefined,
    })
    await expect(fallback.uniqueTabClientId()).resolves.toBe("page-only")
    fallback.dispose()
  })
})
