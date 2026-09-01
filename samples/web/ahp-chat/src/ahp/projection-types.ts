import {
  ResponsePartKind,
  ToolCallStatus,
  ToolResultContentType,
  type ConfirmationOptionKind,
  type ToolCallConfirmationReason,
} from "@microsoft/agent-host-protocol"

export interface ToolConfirmationOptionView {
  id: string
  label: string
  kind: ConfirmationOptionKind
}

export interface ToolResultContentView {
  type: ToolResultContentType
  preview: string
}

export interface ToolCallView {
  toolCallId: string
  name: string
  displayName: string
  status: ToolCallStatus
  intention?: string
  input?: string
  resultContent: readonly ToolResultContentView[]
  resultMessage?: string
  resultSuccess?: boolean
  resultError?: string
  options: readonly ToolConfirmationOptionView[]
  confirmationTitle?: string
  confirmed?: ToolCallConfirmationReason
  selectedOptionId?: string
  selectedOptionLabel?: string
}

export interface MarkdownPartView {
  kind: ResponsePartKind.Markdown
  content: string
}

export interface ReasoningPartView {
  kind: ResponsePartKind.Reasoning
  content: string
}

export interface SystemNotificationPartView {
  kind: ResponsePartKind.SystemNotification
  content: string
}

export interface ContentReferencePartView {
  kind: ResponsePartKind.ContentRef
  uri: string
}

export interface InputRequestPartView {
  kind: ResponsePartKind.InputRequest
  message: string
}

export interface ToolCallPartView {
  kind: ResponsePartKind.ToolCall
  tool: ToolCallView
}

export type ConversationPartView =
  | MarkdownPartView
  | ReasoningPartView
  | SystemNotificationPartView
  | ContentReferencePartView
  | InputRequestPartView
  | ToolCallPartView

export interface ConversationTurn {
  id: string
  userText: string
  assistantText: string
  responseParts: readonly ConversationPartView[]
  state: "complete" | "cancelled" | "error" | "streaming"
  error?: string
}

export function freezeView<T>(value: T): T {
  if (!value || typeof value !== "object" || Object.isFrozen(value)) return value
  for (const child of Object.values(value)) freezeView(child)
  return Object.freeze(value)
}
