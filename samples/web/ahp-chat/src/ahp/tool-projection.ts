import {
  ToolCallStatus,
  ToolResultContentType,
  type ToolCallState,
  type ToolResultContent,
} from "@microsoft/agent-host-protocol"

import type { ToolCallView, ToolResultContentView } from "./projection-types"

function plainText(value: string | { markdown: string }): string {
  return typeof value === "string" ? value : value.markdown
}

function preview(value: string): string {
  let plain = ""
  for (let index = 0; index < value.length; index += 1) {
    if (value.charCodeAt(index) === 27) {
      if (value[index + 1] === "[") {
        index += 1
        while (index + 1 < value.length) {
          index += 1
          const code = value.charCodeAt(index)
          if (code >= 64 && code <= 126) break
        }
      }
      continue
    }
    if (value[index] !== "\0") plain += value[index]
  }
  return plain.length > 1_200 ? `${plain.slice(0, 1_200)}…` : plain
}

function inputText(tool: ToolCallState): string | undefined {
  const input = tool.status === ToolCallStatus.Streaming ? tool.partialInput : tool.toolInput
  if (!input) return undefined
  return typeof input === "string" ? input : `Content: ${input.uri}`
}

function projectResultContent(content: ToolResultContent): ToolResultContentView {
  switch (content.type) {
    case ToolResultContentType.Text:
      return { type: content.type, preview: preview(content.text) }
    case ToolResultContentType.Terminal: {
      const status = content.result?.exitCode === undefined ? "" : ` (exit ${content.result.exitCode})`
      return { type: content.type, preview: preview(`${content.title}${status}\n${content.result?.preview ?? ""}`) }
    }
    case ToolResultContentType.FileEdit: {
      const file = content.after?.uri ?? content.before?.uri ?? "unknown file"
      const changes = content.diff ? ` (+${content.diff.added ?? 0}/-${content.diff.removed ?? 0})` : ""
      return { type: content.type, preview: preview(`Changed ${file}${changes}`) }
    }
    case ToolResultContentType.Resource:
      return { type: content.type, preview: preview(`Resource: ${content.uri}`) }
    case ToolResultContentType.EmbeddedResource:
      return { type: content.type, preview: preview(`Embedded ${content.contentType} result`) }
    default:
      return { type: content.type, preview: "Tool result available." }
  }
}

function resultContent(tool: ToolCallState): readonly ToolResultContentView[] {
  switch (tool.status) {
    case ToolCallStatus.Running:
    case ToolCallStatus.AuthRequired:
    case ToolCallStatus.PendingResultConfirmation:
    case ToolCallStatus.Completed:
      return (tool.content ?? []).map(projectResultContent)
    default:
      return []
  }
}

function selectedOption(tool: ToolCallState) {
  switch (tool.status) {
    case ToolCallStatus.Running:
    case ToolCallStatus.AuthRequired:
    case ToolCallStatus.PendingResultConfirmation:
    case ToolCallStatus.Completed:
    case ToolCallStatus.Cancelled:
      return tool.selectedOption
    default:
      return undefined
  }
}

export function projectToolCall(tool: ToolCallState): ToolCallView {
  const hasResult = tool.status === ToolCallStatus.Completed
    || tool.status === ToolCallStatus.PendingResultConfirmation
  const option = selectedOption(tool)
  return {
    toolCallId: tool.toolCallId,
    name: tool.toolName,
    displayName: tool.displayName,
    status: tool.status,
    intention: tool.intention,
    input: inputText(tool),
    resultContent: resultContent(tool),
    resultMessage: hasResult ? plainText(tool.pastTenseMessage) : undefined,
    resultSuccess: hasResult ? tool.success : undefined,
    resultError: hasResult ? tool.error?.message : undefined,
    options: tool.status === ToolCallStatus.PendingConfirmation
      ? (tool.options ?? []).map(({ id, label, kind }) => ({ id, label, kind }))
      : [],
    confirmationTitle: tool.status === ToolCallStatus.PendingConfirmation
      ? (tool.confirmationTitle && plainText(tool.confirmationTitle))
      : undefined,
    confirmed: tool.status === ToolCallStatus.Running
      || tool.status === ToolCallStatus.AuthRequired
      || hasResult
      ? tool.confirmed
      : undefined,
    selectedOptionId: option?.id,
    selectedOptionLabel: option?.label,
  }
}
