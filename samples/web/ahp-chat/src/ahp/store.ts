import {
  ResponsePartKind,
  ToolCallStatus,
  chatReducer,
  type ActionEnvelope,
  type ChatAction,
  type ChatState,
  type SessionState,
  type SessionSummary,
  type Snapshot,
  type URI,
} from "@microsoft/agent-host-protocol"
import { AhpStateMirror, type SubscriptionEvent } from "@microsoft/agent-host-protocol/client"

import {
  projectChat,
  projectSession,
  projectSessionList,
  type ChatView,
  type SelectedSessionView,
  type SessionListItem,
} from "./projection"
import { freezeView } from "./projection-types"
import { projectModels, type ModelView } from "./model-config"

export type ConnectionPhase = "disconnected" | "connecting" | "initializing" | "connected" | "reconnecting" | "error"

export interface ConnectionView {
  phase: ConnectionPhase
  address?: string
  protocolVersion?: string
  error?: string
  reconnectAttempt?: number
}

export interface AppSnapshot {
  connection: ConnectionView
  models: readonly ModelView[]
  sessions: readonly SessionListItem[]
  selectedSession?: SelectedSessionView
  selectedChat?: ChatView
  notice?: string
}

const rootUri = "ahp-root://" as const
const initialConnection: ConnectionView = { phase: "disconnected" }

export class AhpStore {
  private mirror = new AhpStateMirror()
  private readonly chats = new Map<URI, ChatState>()
  private readonly summaries = new Map<URI, SessionSummary>()
  private readonly listeners = new Set<() => void>()
  private selectedSessionUri?: URI
  private selectedChatUri?: URI
  private connection = initialConnection
  private notice?: string
  private lastServerSeq = 0
  private snapshot: AppSnapshot = this.createSnapshot()

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  getSnapshot = (): AppSnapshot => this.snapshot

  reset(): void {
    this.mirror = new AhpStateMirror()
    this.chats.clear()
    this.summaries.clear()
    this.selectedSessionUri = undefined
    this.selectedChatUri = undefined
    this.lastServerSeq = 0
    this.notice = undefined
    this.publish()
  }

  setConnection(connection: ConnectionView): void {
    this.connection = Object.freeze({ ...connection })
    this.publish()
  }

  setNotice(notice?: string): void {
    this.notice = notice
    this.publish()
  }

  setSelection(sessionUri?: URI, chatUri?: URI): void {
    this.selectedSessionUri = sessionUri
    this.selectedChatUri = chatUri
    this.publish()
  }

  setSessionList(sessions: readonly SessionSummary[]): void {
    this.summaries.clear()
    for (const session of sessions) this.summaries.set(session.resource, session)
    this.publish()
  }

  getSession(uri: URI): SessionState | undefined {
    return this.mirror.getSession(uri)
  }

  getChat(uri: URI): ChatState | undefined {
    return this.chats.get(uri)
  }

  get lastSeenServerSeq(): number {
    return this.lastServerSeq
  }

  applySnapshot(snapshot: Snapshot): void {
    this.lastServerSeq = Math.max(this.lastServerSeq, snapshot.fromSeq)
    if (snapshot.resource.startsWith("ahp-chat:")) {
      this.chats.set(snapshot.resource, snapshot.state as ChatState)
    } else {
      this.mirror.applySnapshot(snapshot)
    }
    this.publish()
  }

  applyEvent(event: SubscriptionEvent): void {
    if (event.type === "action") {
      this.applyAction(event.params)
      return
    }
    if (event.type === "sessionAdded") {
      this.summaries.set(event.params.summary.resource, event.params.summary)
    } else if (event.type === "sessionRemoved") {
      this.summaries.delete(event.params.session)
      if (this.selectedSessionUri === event.params.session) this.setSelection()
    } else if (event.type === "sessionSummaryChanged") {
      const existing = this.summaries.get(event.params.session)
      if (existing) this.summaries.set(event.params.session, { ...existing, ...event.params.changes })
    }
    this.publish()
  }

  private applyAction(envelope: ActionEnvelope): void {
    if (envelope.serverSeq <= this.lastServerSeq) return
    this.lastServerSeq = envelope.serverSeq
    if (envelope.rejectionReason) {
      this.notice = `Host rejected the action: ${envelope.rejectionReason}`
      this.publish()
      return
    }
    if (envelope.channel.startsWith("ahp-chat:")) {
      const current = this.chats.get(envelope.channel)
      if (current) this.chats.set(envelope.channel, chatReducer(current, envelope.action as ChatAction))
    } else {
      this.mirror.apply(envelope)
    }
    this.publish()
  }

  private createSnapshot(): AppSnapshot {
    const sessionUri = this.selectedSessionUri
    const selectedSession = sessionUri ? this.mirror.getSession(sessionUri) : undefined
    const sessionView = selectedSession && sessionUri
      ? { ...projectSession(selectedSession), resource: sessionUri }
      : undefined
    const chat = this.selectedChatUri ? this.chats.get(this.selectedChatUri) : undefined
    const chatView = chat ? projectChat(chat) : undefined
    return freezeView({
      connection: { ...this.connection },
      models: projectModels(this.mirror.root),
      sessions: projectSessionList(this.summaries.values()),
      selectedSession: sessionView,
      selectedChat: chatView,
      notice: this.notice,
    })
  }

  private publish(): void {
    this.snapshot = this.createSnapshot()
    for (const listener of this.listeners) listener()
  }

  get rootChannel(): typeof rootUri {
    return rootUri
  }

  isToolCallPendingConfirmation(chatUri: URI, turnId: string, toolCallId: string): boolean {
    const chat = this.chats.get(chatUri)
    const turn = chat?.activeTurn?.id === turnId
      ? chat.activeTurn
      : chat?.turns.find((candidate) => candidate.id === turnId)
    const toolCall = turn?.responseParts.find((part) =>
      part.kind === ResponsePartKind.ToolCall && part.toolCall.toolCallId === toolCallId)
    return toolCall?.kind === ResponsePartKind.ToolCall
      && toolCall.toolCall.status === ToolCallStatus.PendingConfirmation
  }
}
