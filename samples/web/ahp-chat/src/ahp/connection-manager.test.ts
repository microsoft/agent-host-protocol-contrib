import { InMemoryTransport, type AhpTransport } from "@microsoft/agent-host-protocol/client"
import { describe, expect, it, vi } from "vitest"

import { ConnectionManager } from "./connection-manager"

async function readMessage(transport: AhpTransport): Promise<{ id?: number; method?: string; params?: Record<string, unknown> }> {
  const frame = await transport.recv()
  if (!frame || frame.kind !== "text") throw new Error("Expected a JSON text frame")
  return JSON.parse(frame.text)
}

function reply(transport: AhpTransport, request: { id?: number }, result: unknown): void {
  transport.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, result }))
}

function snapshot(resource: string): { resource: string; state: Record<string, never>; fromSeq: number } {
  return { resource, state: {}, fromSeq: 0 }
}

describe("ConnectionManager", () => {
  it("closes a socket that resolves after disconnecting a stale connection attempt", async () => {
    let resolveTransport: ((transport: AhpTransport) => void) | undefined
    let opened: (() => void) | undefined
    const transportPromise = new Promise<AhpTransport>((resolve) => {
      resolveTransport = resolve
    })
    const openedPromise = new Promise<void>((resolve) => {
      opened = resolve
    })
    const transport: AhpTransport = {
      send: vi.fn(),
      recv: vi.fn(async () => null),
      close: vi.fn(async () => {}),
    }
    const manager = new ConnectionManager("test-client", {
      onConnection: vi.fn(),
      onSnapshot: vi.fn(),
      onEvent: vi.fn(),
      onReconnected: vi.fn(),
    }, async () => {
      opened?.()
      return transportPromise
    })

    const connecting = manager.connect("ws://example.test/ahp")
    await openedPromise
    await manager.disconnect()
    resolveTransport?.(transport)

    await expect(connecting).resolves.toBe(false)
    expect(transport.close).toHaveBeenCalledOnce()
  })

  it("unsubscribes stale resources and reconnects only root plus the active session and chat", async () => {
    vi.useFakeTimers()
    const [firstClient, firstServer] = InMemoryTransport.pair()
    const [secondClient, secondServer] = InMemoryTransport.pair()
    const transports = [firstClient, secondClient]
    const manager = new ConnectionManager("test-client", {
      onConnection: vi.fn(),
      onSnapshot: vi.fn(),
      onEvent: vi.fn(),
      onReconnected: vi.fn(),
    }, async () => {
      const transport = transports.shift()
      if (!transport) throw new Error("No test transport available")
      return transport
    })
    const sessionA = "ahp-session:/a"
    const chatA = "ahp-chat:/a"
    const sessionB = "ahp-session:/b"
    const chatB = "ahp-chat:/b"

    const connecting = manager.connect("ws://example.test/ahp")
    const initialize = await readMessage(firstServer)
    expect(initialize.method).toBe("initialize")
    reply(firstServer, initialize, { protocolVersion: "1.0.0", serverSeq: 0, snapshots: [] })
    await expect(connecting).resolves.toBe(true)

    await manager.reconcileActiveSubscriptions(sessionA)
    const subscribeSessionA = manager.subscribe(sessionA)
    reply(firstServer, await readMessage(firstServer), { snapshot: snapshot(sessionA) })
    await subscribeSessionA
    await manager.reconcileActiveSubscriptions(sessionA, chatA)
    const subscribeChatA = manager.subscribe(chatA)
    reply(firstServer, await readMessage(firstServer), { snapshot: snapshot(chatA) })
    await subscribeChatA

    await manager.reconcileActiveSubscriptions(sessionB, chatB)
    const staleNotifications = [await readMessage(firstServer), await readMessage(firstServer)]
    expect(staleNotifications.map((message) => message.method)).toEqual(["unsubscribe", "unsubscribe"])
    expect(new Set(staleNotifications.map((message) => message.params?.channel))).toEqual(new Set([sessionA, chatA]))

    const subscribeSessionB = manager.subscribe(sessionB)
    reply(firstServer, await readMessage(firstServer), { snapshot: snapshot(sessionB) })
    await subscribeSessionB
    const subscribeChatB = manager.subscribe(chatB)
    reply(firstServer, await readMessage(firstServer), { snapshot: snapshot(chatB) })
    await subscribeChatB

    await firstServer.close()
    await vi.advanceTimersByTimeAsync(1_000)
    const reconnect = await readMessage(secondServer)
    expect(reconnect.method).toBe("reconnect")
    expect(new Set(reconnect.params?.subscriptions as string[])).toEqual(new Set(["ahp-root://", sessionB, chatB]))
    reply(secondServer, reconnect, { type: "replay", actions: [], missing: [] })

    await manager.disconnect()
    vi.useRealTimers()
  })
})
