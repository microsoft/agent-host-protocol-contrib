import {
  ActionType,
  ConfirmationOptionKind,
  MessageKind,
  ToolCallCancellationReason,
  ToolCallConfirmationReason,
  type ChatToolCallConfirmedAction,
  type URI,
} from "@microsoft/agent-host-protocol"
import type { AhpClient } from "@microsoft/agent-host-protocol/client"

import { ConnectionManager } from "./connection-manager"
import type { ConnectionCallbacks } from "./connection-events"
import { displayError } from "./errors"
import { persistentClientId, resourceUri, turnId } from "./ids"
import { ConnectionEpoch } from "./lifecycle"
import { AhpStore, type AppSnapshot } from "./store"
import { saveLastSuccessfulUrl, validateWebSocketUrl } from "./runtime-config"
import { saveLastWorkspaceInput, workspaceInputToFileUri } from "./workspace"

export interface AhpConnection {
  readonly activeClient?: Pick<AhpClient, "dispatch" | "request">
  connect(address: string): Promise<boolean>
  disconnect(): Promise<void>
  refresh(uri: URI): Promise<void>
  reconcileActiveSubscriptions(session?: URI, chat?: URI): Promise<void>
  setLastSeenServerSeq(sequence: number): void
  subscribe(uri: URI): Promise<void>
}

export type AhpConnectionFactory = (callbacks: ConnectionCallbacks) => AhpConnection

export interface ToolConfirmation {
  id: string
  kind: ConfirmationOptionKind
}

export class AhpService {
  readonly store = new AhpStore()
  private readonly selectionEpoch = new ConnectionEpoch()
  private readonly connection: AhpConnection
  private readonly submittedToolConfirmations = new Map<string, { chat: URI; turnId: string; toolCallId: string }>()

  constructor(connectionFactory: AhpConnectionFactory = (callbacks) => new ConnectionManager(persistentClientId(), callbacks)) {
    this.connection = connectionFactory({
      onConnection: (connection) => this.store.setConnection(connection),
      onSnapshot: (snapshot) => {
        this.store.applySnapshot(snapshot)
        this.pruneSubmittedToolConfirmations()
      },
      onEvent: (event) => {
        this.store.applyEvent(event)
        this.pruneSubmittedToolConfirmations()
        this.connection.setLastSeenServerSeq(this.store.lastSeenServerSeq)
      },
      onReconnected: () => void this.refreshSessions(),
    })
  }

  getSnapshot = (): AppSnapshot => this.store.getSnapshot()

  subscribe = (listener: () => void): (() => void) => this.store.subscribe(listener)

  async connect(input: string): Promise<void> {
    let address: string
    try {
      address = validateWebSocketUrl(input)
    } catch (error) {
      this.store.setConnection({ phase: "error", error: displayError(error) })
      return
    }
    this.selectionEpoch.invalidate()
    this.store.reset()
    const connected = await this.connection.connect(address)
    if (!connected) return
    saveLastSuccessfulUrl(address)
    this.connection.setLastSeenServerSeq(this.store.lastSeenServerSeq)
    await this.refreshSessions()
  }

  async disconnect(): Promise<void> {
    this.selectionEpoch.invalidate()
    await this.connection.disconnect()
    this.store.reset()
  }

  async refreshSessions(): Promise<void> {
    const client = this.connection.activeClient
    if (!client) return
    try {
      const sessions = []
      let cursor: string | undefined
      do {
        const page = await client.request("listSessions", {
          channel: this.store.rootChannel,
          limit: 100,
          cursor,
        })
        if (this.connection.activeClient !== client) return
        sessions.push(...page.items)
        cursor = page.nextCursor
      } while (cursor)
      if (this.connection.activeClient !== client) return
      this.store.setSessionList(sessions)
      this.store.setNotice(undefined)
    } catch (error) {
      if (this.connection.activeClient === client) this.store.setNotice(displayError(error))
    }
  }

  async createSession(workspaceInput = ""): Promise<boolean> {
    const token = this.selectionEpoch.begin()
    try {
      const session = resourceUri("session")
      const workingDirectories = workspaceInput.trim() ? [workspaceInputToFileUri(workspaceInput)] : undefined
      await this.requireClient().request("createSession", {
        channel: session,
        provider: "openai",
        ...(workingDirectories ? { workingDirectories } : {}),
      })
      if (!this.isCurrentSelection(token)) return false
      saveLastWorkspaceInput(workspaceInput)
      await this.refreshSessions()
      if (this.isCurrentSelection(token)) await this.selectSession(session, token)
      return true
    } catch (error) {
      if (this.isCurrentSelection(token)) this.store.setNotice(displayError(error))
      return false
    }
  }

  async createChat(): Promise<void> {
    const session = this.store.getSnapshot().selectedSession?.resource
    if (!session) return
    const token = this.selectionEpoch.begin()
    try {
      const chat = resourceUri("chat")
      await this.requireClient().request("createChat", { channel: session, chat })
      if (!this.isCurrentSelection(token)) return
      await this.connection.refresh(session)
      if (this.isCurrentSelection(token)) await this.selectChat(session, chat, token)
    } catch (error) {
      if (this.isCurrentSelection(token)) this.store.setNotice(displayError(error))
    }
  }

  async selectSession(session: URI, token = this.selectionEpoch.begin()): Promise<void> {
    if (!this.isCurrentSelection(token)) return
    this.store.setSelection(session)
    try {
      await this.connection.reconcileActiveSubscriptions(session)
      if (!this.isCurrentSelection(token)) return
      await this.connection.subscribe(session)
      if (!this.isCurrentSelection(token)) return
      const state = this.store.getSession(session)
      const chat = state?.defaultChat ?? state?.chats[0]?.resource
      if (chat && this.isCurrentSelection(token)) await this.selectChat(session, chat, token)
    } catch (error) {
      if (this.isCurrentSelection(token)) this.store.setNotice(displayError(error))
    }
  }

  async selectChat(session: URI, chat: URI, token = this.selectionEpoch.begin()): Promise<void> {
    if (!this.isCurrentSelection(token)) return
    this.store.setSelection(session, chat)
    try {
      await this.connection.reconcileActiveSubscriptions(session, chat)
      if (!this.isCurrentSelection(token)) return
      await this.connection.subscribe(chat)
      if (this.isCurrentSelection(token)) this.store.setNotice(undefined)
    } catch (error) {
      if (this.isCurrentSelection(token)) this.store.setNotice(displayError(error))
    }
  }

  sendPrompt(text: string): void {
    const chat = this.store.getSnapshot().selectedChat
    if (!chat || chat.activeTurn || !text.trim()) return
    try {
      this.requireClient().dispatch(chat.resource, {
        type: ActionType.ChatTurnStarted,
        turnId: turnId(),
        startedAt: new Date().toISOString(),
        message: { text: text.trim(), origin: { kind: MessageKind.User } },
      })
      this.store.setNotice(undefined)
    } catch (error) {
      this.store.setNotice(displayError(error))
    }
  }

  cancelActiveTurn(): void {
    const chat = this.store.getSnapshot().selectedChat
    if (!chat?.activeTurn) return
    try {
      this.requireClient().dispatch(chat.resource, {
        type: ActionType.ChatTurnCancelled,
        turnId: chat.activeTurn.id,
        duration: 0,
      })
    } catch (error) {
      this.store.setNotice(displayError(error))
    }
  }

  confirmToolCall(chat: URI, turnId: string, toolCallId: string, option: ToolConfirmation): boolean {
    const key = `${chat}\0${turnId}\0${toolCallId}`
    if (this.submittedToolConfirmations.has(key)) return false
    try {
      const action: ChatToolCallConfirmedAction = option.kind === ConfirmationOptionKind.Deny
        ? {
            type: ActionType.ChatToolCallConfirmed,
            turnId,
            toolCallId,
            approved: false as const,
            reason: ToolCallCancellationReason.Denied,
            selectedOptionId: option.id,
          }
        : {
            type: ActionType.ChatToolCallConfirmed,
            turnId,
            toolCallId,
            approved: true as const,
            selectedOptionId: option.id,
            confirmed: option.id === "always-allow-command"
              ? ToolCallConfirmationReason.Setting
              : ToolCallConfirmationReason.UserAction,
          }
      this.requireClient().dispatch(chat, action)
      this.submittedToolConfirmations.set(key, { chat, turnId, toolCallId })
      this.store.setNotice(undefined)
      return true
    } catch (error) {
      this.store.setNotice(displayError(error))
      return false
    }
  }

  private requireClient() {
    const client = this.connection.activeClient
    if (!client) throw new Error("Not connected to an AHP host.")
    return client
  }

  private isCurrentSelection(token: number): boolean {
    return this.selectionEpoch.isCurrent(token)
  }

  private pruneSubmittedToolConfirmations(): void {
    for (const [key, submitted] of this.submittedToolConfirmations) {
      if (!this.store.isToolCallPendingConfirmation(submitted.chat, submitted.turnId, submitted.toolCallId)) {
        this.submittedToolConfirmations.delete(key)
      }
    }
  }
}
