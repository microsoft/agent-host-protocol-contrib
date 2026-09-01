import {
  ActionType,
  MessageKind,
  SessionLifecycle,
  type ChatState,
  type SessionState,
  type URI,
} from "@microsoft/agent-host-protocol"
import { describe, expect, it, vi } from "vitest"

import type { ConnectionCallbacks } from "./connection-events"
import { AhpService, type AhpConnection } from "./service"

const session = "ahp-session:/configured" as URI
const chat = "ahp-chat:/configured" as URI
const selectedModel = { id: "gpt-example", config: { thinkingLevel: "medium" } }

function configuredSession(): SessionState {
  return {
    provider: "openai",
    title: "Configured",
    status: 1,
    lifecycle: SessionLifecycle.Ready,
    activeClients: [],
    chats: [],
    config: { schema: { type: "object", properties: {} }, values: { model: selectedModel } },
  }
}

function connection(request = vi.fn(), dispatch = vi.fn()): AhpConnection {
  return {
    activeClient: { request, dispatch } as unknown as AhpConnection["activeClient"],
    connect: vi.fn(async () => true),
    disconnect: vi.fn(async () => {}),
    refresh: vi.fn(async () => {}),
    reconcileActiveSubscriptions: vi.fn(async () => {}),
    setLastSeenServerSeq: vi.fn(),
    subscribe: vi.fn(async () => {}),
  }
}

describe("session model configuration", () => {
  it("uses typed root requests for resolved configuration and model completions", async () => {
    const request = vi.fn(async (method: string) => method === "resolveSessionConfig"
      ? { schema: { type: "object", properties: {} }, values: { model: selectedModel } }
      : { items: [{ value: "gpt-example", label: "Example model" }] })
    const service = new AhpService(() => connection(request))

    await expect(service.resolveSessionConfig("C:\\work\\sample", { model: selectedModel })).resolves.toMatchObject({
      values: { model: selectedModel },
    })
    expect(request).toHaveBeenLastCalledWith("resolveSessionConfig", {
      channel: "ahp-root://",
      provider: "openai",
      workingDirectory: "file:///C:/work/sample",
      config: { model: selectedModel },
    })

    await expect(service.sessionConfigCompletions("C:\\work\\sample", { model: selectedModel }, "model", "example"))
      .resolves.toEqual([{ value: "gpt-example", label: "Example model" }])
    expect(request).toHaveBeenLastCalledWith("sessionConfigCompletions", {
      channel: "ahp-root://",
      provider: "openai",
      workingDirectory: "file:///C:/work/sample",
      config: { model: selectedModel },
      property: "model",
      query: "example",
    })
  })

  it("passes the resolved model config to createSession", async () => {
    const request = vi.fn(async (method: string) => method === "listSessions" ? { items: [] } : undefined)
    const service = new AhpService(() => connection(request))

    await service.createSession("C:\\work\\sample", { model: selectedModel })

    expect(request).toHaveBeenCalledWith("createSession", {
      channel: expect.stringMatching(/^ahp-session:\//),
      provider: "openai",
      workingDirectories: ["file:///C:/work/sample"],
      config: { model: selectedModel },
    })
  })

  it("dispatches canonical config and turn actions and waits for the host echo", () => {
    const dispatch = vi.fn()
    let callbacks: ConnectionCallbacks | undefined
    const service = new AhpService((next) => {
      callbacks = next
      return connection(vi.fn(), dispatch)
    })
    service.store.applySnapshot({ resource: session, state: configuredSession(), fromSeq: 1 })
    service.store.applySnapshot({
      resource: chat,
      state: { resource: chat, title: "Chat", status: 1, modifiedAt: "", turns: [] } satisfies ChatState,
      fromSeq: 2,
    })
    service.store.setSelection(session, chat)

    expect(service.changeSessionModel({ id: "gpt-example", config: { thinkingLevel: "high" } })).toBe(true)
    expect(dispatch).toHaveBeenLastCalledWith(session, {
      type: ActionType.SessionConfigChanged,
      config: { model: { id: "gpt-example", config: { thinkingLevel: "high" } } },
    })
    expect(service.getSnapshot().pendingSessionConfig).toBe(session)

    service.sendPrompt("Explain this")
    expect(dispatch).toHaveBeenLastCalledWith(chat, {
      type: ActionType.ChatTurnStarted,
      turnId: expect.any(String),
      startedAt: expect.any(String),
      message: { text: "Explain this", origin: { kind: MessageKind.User }, model: selectedModel },
    })

    callbacks?.onEvent({
      type: "action",
      params: {
        channel: session,
        serverSeq: 3,
        origin: undefined,
        action: { type: ActionType.SessionConfigChanged, config: { model: selectedModel } },
        rejectionReason: "Model is unavailable",
      },
    })
    expect(service.getSnapshot().pendingSessionConfig).toBeUndefined()
    expect(service.getSnapshot().notice).toContain("Model is unavailable")
  })

  it("drops an old-host config response after a connection switch", async () => {
    let resolveFirst: ((value: unknown) => void) | undefined
    const first = connection(vi.fn(() => new Promise((resolve) => { resolveFirst = resolve })))
    const second = connection(vi.fn())
    let active = first.activeClient
    const service = new AhpService(() => ({
      ...first,
      get activeClient() { return active },
    }))

    const pending = service.resolveSessionConfig()
    active = second.activeClient
    resolveFirst?.({ schema: { type: "object", properties: {} }, values: { model: selectedModel } })

    await expect(pending).resolves.toBeUndefined()
    expect(service.getSnapshot().notice).toBeUndefined()
  })
})
