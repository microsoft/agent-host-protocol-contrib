import {
  ActionType,
  ConfirmationOptionKind,
  SessionLifecycle,
  type ChatState,
  type SessionState,
  type URI,
} from "@microsoft/agent-host-protocol"
import { describe, expect, it, vi } from "vitest"

import type { ConnectionCallbacks } from "./connection-events"
import { AhpService, type AhpConnection } from "./service"

const sessionA = "ahp-session:/a"
const sessionB = "ahp-session:/b"
const chatA = "ahp-chat:/a"
const chatB = "ahp-chat:/b"

function sessionState(resource: URI, chat: URI): SessionState {
  return {
    provider: "openai",
    title: resource === sessionA ? "Session A" : "Session B",
    status: 1,
    lifecycle: SessionLifecycle.Ready,
    activeClients: [],
    chats: [{ resource: chat, title: "Chat", status: 1, modifiedAt: "2026-08-28T00:00:00Z" }],
    defaultChat: chat,
  }
}

function chatState(resource: URI): ChatState {
  return {
    resource,
    title: "Chat",
    status: 1,
    modifiedAt: "2026-08-28T00:00:00Z",
    turns: [],
  }
}

describe("AhpService selection lifecycle", () => {
  it("keeps a fast B selection when slow A subscription resolves later", async () => {
    let callbacks: ConnectionCallbacks | undefined
    let resolveA: (() => void) | undefined
    let signalAStarted: (() => void) | undefined
    const aSubscribed = new Promise<void>((resolve) => { signalAStarted = resolve })
    const slowA = new Promise<void>((resolve) => { resolveA = resolve })
    const reconcile = vi.fn(async () => {})
    const connection: AhpConnection = {
      connect: vi.fn(async () => true),
      disconnect: vi.fn(async () => {}),
      refresh: vi.fn(async () => {}),
      reconcileActiveSubscriptions: reconcile,
      setLastSeenServerSeq: vi.fn(),
      subscribe: vi.fn(async (uri) => {
        if (uri === sessionA) {
          signalAStarted?.()
          await slowA
          callbacks?.onSnapshot({ resource: sessionA, state: sessionState(sessionA, chatA), fromSeq: 1 })
          return
        }
        if (uri === sessionB) {
          callbacks?.onSnapshot({ resource: sessionB, state: sessionState(sessionB, chatB), fromSeq: 2 })
          return
        }
        callbacks?.onSnapshot({ resource: chatB, state: chatState(chatB), fromSeq: 3 })
      }),
    }
    const service = new AhpService((nextCallbacks) => {
      callbacks = nextCallbacks
      return connection
    })

    const selectA = service.selectSession(sessionA)
    await aSubscribed
    await service.selectSession(sessionB)
    resolveA?.()
    await selectA

    expect(service.getSnapshot().selectedSession?.resource).toBe(sessionB)
    expect(service.getSnapshot().selectedChat?.resource).toBe(chatB)
    expect(reconcile).toHaveBeenLastCalledWith(sessionB, chatB)
    expect(reconcile).not.toHaveBeenCalledWith(sessionA, chatA)
  })

  it("sends one converted workspace URI when creating a session", async () => {
    const request = vi.fn(async (method: string) => method === "listSessions" ? { items: [] } : undefined)
    const connection: AhpConnection = {
      activeClient: { request, dispatch: vi.fn() } as unknown as AhpConnection["activeClient"],
      connect: vi.fn(async () => true),
      disconnect: vi.fn(async () => {}),
      refresh: vi.fn(async () => {}),
      reconcileActiveSubscriptions: vi.fn(async () => {}),
      setLastSeenServerSeq: vi.fn(),
      subscribe: vi.fn(async () => {}),
    }
    const service = new AhpService(() => connection)

    await service.createSession("C:\\work\\sample")

    expect(request).toHaveBeenCalledWith("createSession", {
      channel: expect.stringMatching(/^ahp-session:\//),
      provider: "openai",
      workingDirectories: ["file:///C:/work/sample"],
    })
  })

  it("lets the server select its default workspace when none is entered", async () => {
    const request = vi.fn(async (method: string) => method === "listSessions" ? { items: [] } : undefined)
    const connection: AhpConnection = {
      activeClient: { request, dispatch: vi.fn() } as unknown as AhpConnection["activeClient"],
      connect: vi.fn(async () => true),
      disconnect: vi.fn(async () => {}),
      refresh: vi.fn(async () => {}),
      reconcileActiveSubscriptions: vi.fn(async () => {}),
      setLastSeenServerSeq: vi.fn(),
      subscribe: vi.fn(async () => {}),
    }
    const service = new AhpService(() => connection)

    await service.createSession()

    expect(request).toHaveBeenCalledWith("createSession", {
      channel: expect.stringMatching(/^ahp-session:\//),
      provider: "openai",
    })
  })

  it("dispatches exact confirmation actions once per pending tool call", () => {
    const dispatch = vi.fn()
    const connection: AhpConnection = {
      activeClient: { request: vi.fn(), dispatch } as unknown as AhpConnection["activeClient"],
      connect: vi.fn(async () => true),
      disconnect: vi.fn(async () => {}),
      refresh: vi.fn(async () => {}),
      reconcileActiveSubscriptions: vi.fn(async () => {}),
      setLastSeenServerSeq: vi.fn(),
      subscribe: vi.fn(async () => {}),
    }
    const service = new AhpService(() => connection)

    expect(service.confirmToolCall(chatA, "turn-1", "tool-1", {
      id: "allow-once",
      kind: ConfirmationOptionKind.Approve,
    })).toBe(true)
    expect(service.confirmToolCall(chatA, "turn-1", "tool-1", {
      id: "allow-once",
      kind: ConfirmationOptionKind.Approve,
    })).toBe(false)
    expect(dispatch).toHaveBeenCalledTimes(1)
    expect(dispatch).toHaveBeenCalledWith(chatA, {
      type: ActionType.ChatToolCallConfirmed,
      turnId: "turn-1",
      toolCallId: "tool-1",
      approved: true,
      selectedOptionId: "allow-once",
      confirmed: "user-action",
    })

    expect(service.confirmToolCall(chatA, "turn-2", "tool-2", {
      id: "always-allow-command",
      kind: ConfirmationOptionKind.Approve,
    })).toBe(true)
    expect(service.confirmToolCall(chatA, "turn-3", "tool-3", {
      id: "deny",
      kind: ConfirmationOptionKind.Deny,
    })).toBe(true)
    expect(dispatch).toHaveBeenLastCalledWith(chatA, {
      type: ActionType.ChatToolCallConfirmed,
      turnId: "turn-3",
      toolCallId: "tool-3",
      approved: false,
      selectedOptionId: "deny",
      reason: "denied",
    })
    expect(dispatch).toHaveBeenNthCalledWith(2, chatA, {
      type: ActionType.ChatToolCallConfirmed,
      turnId: "turn-2",
      toolCallId: "tool-2",
      approved: true,
      selectedOptionId: "always-allow-command",
      confirmed: "setting",
    })
  })
})
