import {
  ResponsePartKind,
  TurnState,
  type ChatState,
  type ChatSummary,
  type ResponsePart,
  type SessionState,
  type SessionSummary,
  type StringOrMarkdown,
  type Turn,
} from "@microsoft/agent-host-protocol"

import type {
  ConversationPartView,
  ConversationTurn,
} from "./projection-types"
import { modelSelectionFromConfig, projectSessionConfig, type SessionConfigView } from "./model-config"
import { projectToolCall } from "./tool-projection"

export interface SessionListItem {
  resource: string
  title: string
  provider: string
  status: number
  activity?: string
  modifiedAt: string
  workingDirectories: readonly string[]
}

export interface ChatListItem {
  resource: string
  title: string
  status: number
  activity?: string
}

export interface SelectedSessionView {
  resource: string
  title: string
  lifecycle: string
  chats: readonly ChatListItem[]
  defaultChat?: string
  workingDirectories: readonly string[]
  serverTools: readonly string[]
  config?: SessionConfigView
}

export interface ChatView {
  resource: string
  title: string
  activity?: string
  turns: readonly ConversationTurn[]
  activeTurn?: ConversationTurn
}

function plainText(value: StringOrMarkdown): string {
  return typeof value === "string" ? value : value.markdown
}

export function projectResponseParts(parts: readonly ResponsePart[]): readonly ConversationPartView[] {
  return parts.map((part) => {
    switch (part.kind) {
      case ResponsePartKind.Markdown:
      case ResponsePartKind.Reasoning:
        return { kind: part.kind, content: part.content }
      case ResponsePartKind.SystemNotification:
        return { kind: part.kind, content: plainText(part.content) }
      case ResponsePartKind.ToolCall:
        return { kind: part.kind, tool: projectToolCall(part.toolCall) }
      case ResponsePartKind.ContentRef:
        return { kind: part.kind, uri: part.uri }
      case ResponsePartKind.InputRequest:
        return { kind: part.kind, message: plainText(part.request.message ?? "Input requested.") }
    }
  })
}

export function responseText(parts: readonly ResponsePart[]): string {
  return parts
    .flatMap((part) => {
      if (part.kind === ResponsePartKind.Markdown || part.kind === ResponsePartKind.Reasoning) {
        return [part.content]
      }
      if (part.kind === ResponsePartKind.SystemNotification) return [plainText(part.content)]
      return []
    })
    .join("")
}

function projectTurn(turn: Turn): ConversationTurn {
  return {
    id: turn.id,
    userText: turn.message.text,
    model: modelSelectionFromConfig({ model: turn.message.model }),
    assistantText: responseText(turn.responseParts),
    responseParts: projectResponseParts(turn.responseParts),
    state: turn.state,
    error: turn.error?.message,
  }
}

function projectActiveTurn(chat: ChatState): ConversationTurn | undefined {
  const turn = chat.activeTurn
  if (!turn) return undefined
  return {
    id: turn.id,
    userText: turn.message.text,
    model: modelSelectionFromConfig({ model: turn.message.model }),
    assistantText: responseText(turn.responseParts),
    responseParts: projectResponseParts(turn.responseParts),
    state: "streaming",
  }
}

function projectChatSummary(chat: ChatSummary): ChatListItem {
  return {
    resource: chat.resource,
    title: chat.title,
    status: chat.status,
    activity: chat.activity,
  }
}

export function projectChat(chat: ChatState): ChatView {
  return {
    resource: chat.resource,
    title: chat.title,
    activity: chat.activity,
    turns: chat.turns.map(projectTurn),
    activeTurn: projectActiveTurn(chat),
  }
}

export function projectSession(session: SessionState): SelectedSessionView {
  return {
    resource: "",
    title: session.title,
    lifecycle: session.lifecycle,
    chats: session.chats.map(projectChatSummary),
    defaultChat: session.defaultChat,
    workingDirectories: [...(session.workingDirectories ?? [])],
    serverTools: (session.serverTools ?? []).map((tool) => tool.name),
    config: projectSessionConfig(session.config),
  }
}

export function projectSessionList(summaries: Iterable<SessionSummary>): readonly SessionListItem[] {
  return [...summaries]
    .map((session) => ({
      resource: session.resource,
      title: session.title,
      provider: session.provider,
      status: session.status,
      activity: session.activity,
      modifiedAt: session.modifiedAt,
      workingDirectories: [...(session.workingDirectories ?? [])],
    }))
    .sort((left, right) => right.modifiedAt.localeCompare(left.modifiedAt))
}

export function isTerminalTurn(turn: ConversationTurn): boolean {
  return turn.state === TurnState.Complete || turn.state === TurnState.Cancelled || turn.state === TurnState.Error
}
