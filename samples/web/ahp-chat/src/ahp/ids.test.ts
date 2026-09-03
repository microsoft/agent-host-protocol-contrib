import { afterEach, describe, expect, it, vi } from "vitest"

import { tabClientId } from "./ids"

class MemoryStorage {
  private readonly values = new Map<string, string>()

  getItem(key: string): string | null {
    return this.values.get(key) ?? null
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value)
  }
}

describe("tab client ID", () => {
  afterEach(() => vi.unstubAllGlobals())

  it("uses distinct session-storage IDs per tab", () => {
    const sharedLocalStorage = new MemoryStorage()
    sharedLocalStorage.setItem("ahp-chat.client-id", "legacy-shared-id")
    const firstTab = new MemoryStorage()
    vi.stubGlobal("window", { sessionStorage: firstTab, localStorage: sharedLocalStorage })
    const firstId = tabClientId()
    expect(tabClientId()).toBe(firstId)

    const secondTab = new MemoryStorage()
    vi.stubGlobal("window", { sessionStorage: secondTab, localStorage: sharedLocalStorage })
    const secondId = tabClientId()
    expect(tabClientId()).toBe(secondId)
    expect(secondId).not.toBe(firstId)
    expect(firstId).not.toBe("legacy-shared-id")
  })

  it("uses one in-memory ID when session storage is unavailable", () => {
    vi.stubGlobal("window", {
      get sessionStorage(): never { throw new Error("Storage is unavailable") },
    })
    expect(tabClientId()).toBe(tabClientId())
  })
})
