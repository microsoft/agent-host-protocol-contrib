import {
  ActionType,
  ConfirmationOptionKind,
  MessageKind,
  ResponsePartKind,
  chatReducer,
  type ChatState,
} from "@microsoft/agent-host-protocol"
import { describe, expect, it } from "vitest"

import { projectChat } from "./projection"
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
  })
})
