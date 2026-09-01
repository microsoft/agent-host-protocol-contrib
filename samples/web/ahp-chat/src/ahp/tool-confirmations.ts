import {
  ActionType,
  ConfirmationOptionKind,
  ToolCallCancellationReason,
  ToolCallConfirmationReason,
  type ChatToolCallConfirmedAction,
  type URI,
} from "@microsoft/agent-host-protocol"
import type { AhpClient } from "@microsoft/agent-host-protocol/client"

import { AhpStore } from "./store"

export interface ToolConfirmation {
  id: string
  kind: ConfirmationOptionKind
}

export class ToolConfirmations {
  private readonly submitted = new Map<string, { chat: URI; turnId: string; toolCallId: string }>()

  submit(client: Pick<AhpClient, "dispatch">, chat: URI, turnId: string, toolCallId: string, option: ToolConfirmation): boolean {
    const key = `${chat}\0${turnId}\0${toolCallId}`
    if (this.submitted.has(key)) return false
    const action: ChatToolCallConfirmedAction = option.kind === ConfirmationOptionKind.Deny
      ? { type: ActionType.ChatToolCallConfirmed, turnId, toolCallId, approved: false, reason: ToolCallCancellationReason.Denied, selectedOptionId: option.id }
      : {
          type: ActionType.ChatToolCallConfirmed,
          turnId,
          toolCallId,
          approved: true,
          selectedOptionId: option.id,
          confirmed: option.id === "always-allow-command" ? ToolCallConfirmationReason.Setting : ToolCallConfirmationReason.UserAction,
        }
    client.dispatch(chat, action)
    this.submitted.set(key, { chat, turnId, toolCallId })
    return true
  }

  prune(store: AhpStore): void {
    for (const [key, value] of this.submitted) {
      if (!store.isToolCallPendingConfirmation(value.chat, value.turnId, value.toolCallId)) this.submitted.delete(key)
    }
  }
}
