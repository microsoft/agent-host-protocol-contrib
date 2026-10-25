import {
  ActionType,
  ConfirmationOptionKind,
  MessageKind,
  ResponsePartKind,
  TurnState,
  chatReducer,
  type ChatState,
} from "@microsoft/agent-host-protocol"
import { describe, expect, it } from "vitest"

import { projectChat, visibleResponseParts } from "./projection"
import { AhpStore } from "./store"

const baseChat: ChatState = {
  resource: "ahp-chat:/demo",
  title: "Demo",
  status: 1,
  modifiedAt: "2026-08-28T00:00:00.000Z",
  turns: [],
}

describe("chat projection", () => {
  it("projects reducer-driven streaming deltas without mutating the prior state", () => {
    const started = chatReducer(baseChat, {
      type: ActionType.ChatTurnStarted,
      turnId: "turn-1",
      startedAt: "2026-08-28T00:00:01.000Z",
      message: { text: "Hello", origin: { kind: MessageKind.User } },
    })
    const withPart = chatReducer(started, {
      type: ActionType.ChatResponsePart,
      turnId: "turn-1",
      part: { kind: ResponsePartKind.Markdown, id: "part-1", content: "" },
    })
    const streaming = chatReducer(withPart, {
      type: ActionType.ChatDelta,
      turnId: "turn-1",
      partId: "part-1",
      content: "Hi there",
    })

    expect(baseChat.activeTurn).toBeUndefined()
    expect(projectChat(streaming).activeTurn).toMatchObject({
      id: "turn-1",
      userText: "Hello",
      assistantText: "Hi there",
      state: "streaming",
    })
  })

  it("publishes a fresh immutable React-facing snapshot", () => {
    const store = new AhpStore()
    store.applySnapshot({ resource: baseChat.resource, state: baseChat, fromSeq: 0 })
    store.setSelection(undefined, baseChat.resource)

    const before = store.getSnapshot()
    store.applyEvent({
      type: "action",
      params: {
        channel: baseChat.resource,
        serverSeq: 1,
        origin: undefined,
        action: {
          type: ActionType.ChatTurnStarted,
          turnId: "turn-1",
          startedAt: "2026-08-28T00:00:01.000Z",
          message: { text: "Hello", origin: { kind: MessageKind.User } },
        },
      },
    })
    const after = store.getSnapshot()

    expect(after).not.toBe(before)
    expect(Object.isFrozen(after)).toBe(true)
    expect(Object.isFrozen(after.selectedChat?.turns)).toBe(true)
    expect(Object.isFrozen(after.selectedChat?.activeTurn)).toBe(true)
    expect(Object.isFrozen(after.selectedChat?.activeTurn?.responseParts)).toBe(true)
    expect(after.selectedChat?.activeTurn?.userText).toBe("Hello")
  })

  it("projects tool parts in response order with the host's pending options", () => {
    const started = chatReducer(baseChat, {
      type: ActionType.ChatTurnStarted,
      turnId: "turn-1",
      startedAt: "2026-08-28T00:00:01.000Z",
      message: { text: "Hello", origin: { kind: MessageKind.User } },
    })
    const beforeTool = chatReducer(started, {
      type: ActionType.ChatResponsePart,
      turnId: "turn-1",
      part: { kind: ResponsePartKind.Markdown, id: "before", content: "Before " },
    })
    const toolStarted = chatReducer(beforeTool, {
      type: ActionType.ChatToolCallStart,
      turnId: "turn-1",
      toolCallId: "shell-1",
      toolName: "run_shell",
      displayName: "Run shell",
      intention: "List files",
    })
    const pendingTool = chatReducer(toolStarted, {
      type: ActionType.ChatToolCallReady,
      turnId: "turn-1",
      toolCallId: "shell-1",
      invocationMessage: "Run dir",
      toolInput: "dir",
      confirmationTitle: "Run command",
      options: [
        { id: "allow-once", label: "Allow once", kind: ConfirmationOptionKind.Approve },
        { id: "always-allow-command", label: "Always allow dir", kind: ConfirmationOptionKind.Approve },
        { id: "deny", label: "Deny", kind: ConfirmationOptionKind.Deny },
      ],
    })
    const ordered = chatReducer(pendingTool, {
      type: ActionType.ChatResponsePart,
      turnId: "turn-1",
      part: { kind: ResponsePartKind.Markdown, id: "after", content: "After" },
    })

    const parts = projectChat(ordered).activeTurn?.responseParts
    expect(parts?.map((part) => part.kind)).toEqual([
      ResponsePartKind.Markdown,
      ResponsePartKind.ToolCall,
      ResponsePartKind.Markdown,
    ])
    expect(parts?.[1]).toMatchObject({
      kind: ResponsePartKind.ToolCall,
      tool: {
        name: "run_shell",
        input: "dir",
        confirmationTitle: "Run command",
        options: [
          { id: "allow-once", label: "Allow once", kind: ConfirmationOptionKind.Approve },
          { id: "always-allow-command", label: "Always allow dir", kind: ConfirmationOptionKind.Approve },
          { id: "deny", label: "Deny", kind: ConfirmationOptionKind.Deny },
        ],
      },
    })
    expect(visibleResponseParts(parts ?? []).map((part) => part.kind)).toEqual([
      ResponsePartKind.Markdown,
      ResponsePartKind.ToolCall,
      ResponsePartKind.Markdown,
    ])
  })

  it("projects AHP 1.0 turn errors in response order", () => {
    const started = chatReducer(baseChat, {
      type: ActionType.ChatTurnStarted,
      turnId: "turn-1",
      startedAt: "2026-08-28T00:00:01.000Z",
      message: { text: "Hello", origin: { kind: MessageKind.User } },
    })
    const failed = chatReducer(started, {
      type: ActionType.ChatError,
      turnId: "turn-1",
      duration: 10,
      part: {
        kind: ResponsePartKind.Error,
        error: { errorType: "model", message: "Model unavailable" },
        resumable: true,
      },
    })

    expect(projectChat(failed).turns[0]).toMatchObject({
      state: TurnState.Error,
      responseParts: [{
        kind: ResponsePartKind.Error,
        message: "Model unavailable",
        resumable: true,
      }],
    })
  })

  it("keeps tools between markdown while skipping empty compatibility parts", () => {
    const started = chatReducer(baseChat, {
      type: ActionType.ChatTurnStarted,
      turnId: "turn-1",
      startedAt: "2026-08-28T00:00:01.000Z",
      message: { text: "Hello", origin: { kind: MessageKind.User } },
    })
    const empty = chatReducer(started, {
      type: ActionType.ChatResponsePart,
      turnId: "turn-1",
      part: { kind: ResponsePartKind.Markdown, id: "empty", content: "" },
    })
    const tool = chatReducer(empty, {
      type: ActionType.ChatToolCallStart,
      turnId: "turn-1",
      toolCallId: "shell-1",
      toolName: "run_shell",
      displayName: "Run shell",
    })
    const final = chatReducer(tool, {
      type: ActionType.ChatResponsePart,
      turnId: "turn-1",
      part: { kind: ResponsePartKind.Markdown, id: "final", content: "Done" },
    })

    const parts = projectChat(final).activeTurn?.responseParts ?? []
    expect(parts.map((part) => part.kind)).toEqual([
      ResponsePartKind.Markdown,
      ResponsePartKind.ToolCall,
      ResponsePartKind.Markdown,
    ])
    expect(visibleResponseParts(parts).map((part) => part.kind)).toEqual([
      ResponsePartKind.ToolCall,
      ResponsePartKind.Markdown,
    ])
  })

  it("projects and freezes a shared draft including unknown model config", () => {
    const store = new AhpStore()
    const chat: ChatState = {
      ...baseChat,
      draft: {
        text: "Shared text",
        origin: { kind: MessageKind.User },
        model: { id: "gpt-example", config: { thinkingLevel: "medium", contextSize: 200_000 } },
      },
    }
    store.applySnapshot({ resource: chat.resource, state: chat, fromSeq: 1 })
    store.setSelection(undefined, chat.resource)

    const draft = store.getSnapshot().selectedChat?.draft
    expect(draft).toEqual({
      text: "Shared text",
      model: { id: "gpt-example", config: { thinkingLevel: "medium", contextSize: 200_000 } },
    })
    expect(Object.isFrozen(draft)).toBe(true)
    expect(Object.isFrozen(draft?.model?.config)).toBe(true)
  })

  it("applies authoritative draft changes and clears only when the host clears it", () => {
    const store = new AhpStore()
    store.applySnapshot({ resource: baseChat.resource, state: baseChat, fromSeq: 0 })
    store.setSelection(undefined, baseChat.resource)
    const draft = {
      text: "Remote draft",
      origin: { kind: MessageKind.User },
      model: { id: "gpt-example", config: { contextSize: 200_000 } },
    }
    store.applyEvent({
      type: "action",
      params: {
        channel: baseChat.resource,
        serverSeq: 1,
        origin: { clientId: "other-client", clientSeq: 1 },
        action: { type: ActionType.ChatDraftChanged, draft },
      },
    })
    expect(store.getSnapshot().selectedChat?.draft).toMatchObject({ text: "Remote draft", model: draft.model })
    store.applyEvent({
      type: "action",
      params: {
        channel: baseChat.resource,
        serverSeq: 2,
        origin: { clientId: "local-client", clientSeq: 1 },
        action: {
          type: ActionType.ChatTurnStarted,
          turnId: "rejected",
          startedAt: "2026-08-28T00:00:01.000Z",
          message: draft,
        },
        rejectionReason: "Model unavailable",
      },
    })
    expect(store.getSnapshot().selectedChat?.draft?.text).toBe("Remote draft")
    store.applyEvent({
      type: "action",
      params: {
        channel: baseChat.resource,
        serverSeq: 3,
        origin: { clientId: "other-client", clientSeq: 2 },
        action: { type: ActionType.ChatDraftChanged },
      },
    })
    expect(store.getSnapshot().selectedChat?.draft).toBeUndefined()
  })
})
