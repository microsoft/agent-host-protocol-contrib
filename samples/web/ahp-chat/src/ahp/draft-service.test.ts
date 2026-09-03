import {
  ActionType,
  MessageKind,
  SessionLifecycle,
  TurnState,
  type ChatState,
  type RootState,
  type SessionState,
} from "@microsoft/agent-host-protocol"
import { describe, expect, it, vi } from "vitest"

import { draftExpectedTurnIdMeta } from "./drafts"
import { AhpService, type AhpConnection } from "./service"

const session = "ahp-session:/draft-service"
const chat = "ahp-chat:/draft-service"
const model = { id: "gpt-example" }

function connection(dispatch: ReturnType<typeof vi.fn>): AhpConnection {
  return {
    activeClient: { request: vi.fn(), dispatch } as unknown as AhpConnection["activeClient"],
    connect: vi.fn(async () => true),
    disconnect: vi.fn(async () => {}),
    refresh: vi.fn(async () => {}),
    reconcileActiveSubscriptions: vi.fn(async () => {}),
    setLastSeenServerSeq: vi.fn(),
    subscribe: vi.fn(async () => {}),
  }
}

function seed(service: AhpService, sessionDefault = true): void {
  const root: RootState = {
    agents: [{ provider: "openai", displayName: "OpenAI", description: "", models: [{
      ...model,
      provider: "openai",
      name: "Example",
    }] }],
  }
  const state: SessionState = {
    provider: "openai",
    title: "Session",
    status: 1,
    lifecycle: SessionLifecycle.Ready,
    activeClients: [],
    chats: [],
    config: { schema: { type: "object", properties: {} }, values: sessionDefault ? { model } : {} },
  }
  const chatState: ChatState = { resource: chat, title: "Chat", status: 1, modifiedAt: "", turns: [] }
  service.store.applySnapshot({ resource: "ahp-root://", state: root, fromSeq: 1 })
  service.store.applySnapshot({ resource: session, state, fromSeq: 2 })
  service.store.applySnapshot({ resource: chat, state: chatState, fromSeq: 3 })
  service.store.setSelection(session, chat)
}

describe("draft service actions", () => {
  it("requires a valid draft model when the session has no default", () => {
    const dispatch = vi.fn(() => ({ clientSeq: 1 }))
    const service = new AhpService(() => connection(dispatch))
    seed(service, false)

    expect(service.sendPrompt({ text: "No model" })).toBe(false)
    expect(dispatch).not.toHaveBeenCalled()
    expect(service.getSnapshot().notice).toBe("Choose a model to continue.")
  })

  it("flushes a typed draft before starting a model-selected turn without clearing local state", () => {
    const dispatch = vi.fn(() => ({ clientSeq: 1 }))
    const service = new AhpService(() => connection(dispatch))
    seed(service)
    const draft = { text: "Draft text", model }

    expect(service.updateDraft({ text: "" }, undefined)).toBe(true)
    expect(dispatch).toHaveBeenLastCalledWith(chat, { type: ActionType.ChatDraftChanged })
    expect(service.updateDraft(draft, undefined)).toBe(true)
    expect(dispatch).toHaveBeenLastCalledWith(chat, {
      type: ActionType.ChatDraftChanged,
      draft: {
        text: "Draft text",
        origin: { kind: MessageKind.User },
        model,
        _meta: { [draftExpectedTurnIdMeta]: null },
      },
    })
    expect(service.sendPrompt(draft)).toBe(true)
    expect(dispatch).toHaveBeenLastCalledWith(chat, {
      type: ActionType.ChatTurnStarted,
      turnId: expect.any(String),
      startedAt: expect.any(String),
      message: { text: "Draft text", origin: { kind: MessageKind.User }, model },
    })
  })

  it("rejects a queued draft after a remote turn starts and finishes", () => {
    const dispatch = vi.fn(() => ({ clientSeq: 1 }))
    const service = new AhpService(() => connection(dispatch))
    seed(service)
    const draft = { text: "Stale draft", model }

    service.store.applySnapshot({
      resource: chat,
      fromSeq: 4,
      state: {
        resource: chat,
        title: "Chat",
        status: 1,
        modifiedAt: "",
        turns: [{
          id: "remote-turn",
          message: { text: "Remote prompt", origin: { kind: MessageKind.User } },
          responseParts: [],
          usage: undefined,
          state: TurnState.Complete,
        }],
      },
    })

    expect(service.updateDraft(draft, undefined)).toBe(false)
    expect(dispatch).not.toHaveBeenCalled()
    expect(service.updateDraft(draft, "remote-turn")).toBe(true)
    expect(dispatch).toHaveBeenLastCalledWith(chat, {
      type: ActionType.ChatDraftChanged,
      draft: {
        text: "Stale draft",
        origin: { kind: MessageKind.User },
        model,
        _meta: { [draftExpectedTurnIdMeta]: "remote-turn" },
      },
    })
  })
})
