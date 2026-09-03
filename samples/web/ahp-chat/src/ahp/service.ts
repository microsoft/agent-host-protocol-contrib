import { ActionType, type URI } from "@microsoft/agent-host-protocol"
import type { AhpClient } from "@microsoft/agent-host-protocol/client"
import { ConnectionManager } from "./connection-manager"
import type { ConnectionCallbacks } from "./connection-events"
import { SessionConfigRequests } from "./config-requests"
import { isAvailableModelSelection, modelSelectionFromConfig } from "./model-config"
import { draftMessage, type DraftView } from "./drafts"
import { displayError } from "./errors"
import { resourceUri, tabClientId, turnId } from "./ids"
import { ConnectionEpoch } from "./lifecycle"
import { latestTurnId } from "./projection"
import { AhpStore, type AppSnapshot } from "./store"
import { saveLastSuccessfulUrl, validateWebSocketUrl } from "./runtime-config"
import { ToolConfirmations, type ToolConfirmation } from "./tool-confirmations"
import { saveLastWorkspaceInput, workspaceInputToFileUri } from "./workspace"
import { TabIdentity, type TabIdentityProvider } from "./tab-identity"
export type { ToolConfirmation } from "./tool-confirmations"
export interface AhpConnection {
  readonly activeClient?: Pick<AhpClient, "dispatch" | "request">
  readonly connectionGeneration?: number
  setClientId?(clientId: string): void
  connect(address: string): Promise<boolean>
  disconnect(): Promise<void>
  refresh(uri: URI): Promise<void>
  reconcileActiveSubscriptions(session?: URI, chat?: URI): Promise<void>
  setLastSeenServerSeq(sequence: number): void
  subscribe(uri: URI): Promise<void>
}
export type AhpConnectionFactory = (callbacks: ConnectionCallbacks) => AhpConnection
export class AhpService {
  readonly store = new AhpStore()
  private readonly selectionEpoch = new ConnectionEpoch()
  private readonly connectionEpoch = new ConnectionEpoch()
  private readonly connection: AhpConnection
  private readonly identity: TabIdentityProvider
  private readonly configRequests = new SessionConfigRequests(() => this.connection.activeClient, this.store)
  private readonly toolConfirmations = new ToolConfirmations()
  constructor(connectionFactory?: AhpConnectionFactory, clientId = tabClientId(), identity: TabIdentityProvider = new TabIdentity()) {
    this.identity = identity
    const factory = connectionFactory ?? ((callbacks) => new ConnectionManager(clientId, callbacks))
    this.connection = factory({
      onConnection: (connection) => this.store.setConnection(connection),
      onSnapshot: (snapshot) => { this.store.applySnapshot(snapshot); this.toolConfirmations.prune(this.store) },
      onEvent: (event, connectionGeneration) => {
        if (!this.isCurrentConnection(connectionGeneration)) return
        this.store.applyEvent(event)
        this.toolConfirmations.prune(this.store)
        this.connection.setLastSeenServerSeq(this.store.lastSeenServerSeq)
      },
      onReconnected: () => void this.refreshSessions(),
    })
  }
  getSnapshot = (): AppSnapshot => this.store.getSnapshot(); subscribe = (listener: () => void): (() => void) => this.store.subscribe(listener)
  async connect(input: string): Promise<void> {
    let address: string
    try {
      address = validateWebSocketUrl(input)
    } catch (error) {
      this.store.setConnection({ phase: "error", error: displayError(error) })
      return
    }
    const token = this.connectionEpoch.begin()
    this.selectionEpoch.invalidate(); this.configRequests.invalidate()
    this.store.reset()
    this.store.setConnection({ phase: "connecting", address })
    const clientId = await this.identity.uniqueTabClientId()
    if (!this.connectionEpoch.isCurrent(token)) return
    this.connection.setClientId?.(clientId)
    const connected = await this.connection.connect(address)
    if (!connected || !this.connectionEpoch.isCurrent(token)) return
    saveLastSuccessfulUrl(address)
    this.connection.setLastSeenServerSeq(this.store.lastSeenServerSeq)
    await this.refreshSessions()
  }
  async disconnect(): Promise<void> {
    this.connectionEpoch.invalidate(); this.selectionEpoch.invalidate(); this.configRequests.invalidate()
    await this.connection.disconnect()
    this.store.reset()
  }
  async dispose(): Promise<void> { this.identity.dispose(); await this.disconnect() }
  async refreshSessions(): Promise<void> {
    const client = this.connection.activeClient
    if (!client) return
    try {
      const sessions = []
      let cursor: string | undefined
      do {
        const page = await client.request("listSessions", { channel: this.store.rootChannel, limit: 100, cursor })
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
  resolveSessionConfig = this.configRequests.resolve.bind(this.configRequests)
  sessionConfigCompletions = this.configRequests.completions.bind(this.configRequests)
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
  updateDraft(draft: DraftView | undefined, expectedTurnId: string | undefined): boolean {
    const snapshot = this.store.getSnapshot()
    const chat = snapshot.selectedChat
    if (!chat || chat.activeTurn || latestTurnId(chat) !== expectedTurnId) return false
    try {
      const message = draft && draftMessage(draft, expectedTurnId ?? null)
      this.requireClient().dispatch(chat.resource, {
        type: ActionType.ChatDraftChanged,
        ...(message ? { draft: message } : {}),
      })
      this.store.setNotice(undefined)
      return true
    } catch (error) {
      this.store.setNotice(displayError(error))
      return false
    }
  }
  sendPrompt(draft: DraftView): boolean {
    const snapshot = this.store.getSnapshot()
    const chat = snapshot.selectedChat
    const message = draftMessage(draft)
    const model = draft.model ?? modelSelectionFromConfig(snapshot.selectedSession?.config?.values)
    if (!chat || chat.activeTurn || !message?.text.trim()) return false
    if (snapshot.models.length && !isAvailableModelSelection(snapshot.models, model)) {
      this.store.setNotice("Choose a model to continue.")
      return false
    }
    try {
      this.requireClient().dispatch(chat.resource, {
        type: ActionType.ChatTurnStarted,
        turnId: turnId(),
        startedAt: new Date().toISOString(),
        message: { ...message, text: message.text.trim() },
      })
      this.store.setNotice(undefined)
      return true
    } catch (error) {
      this.store.setNotice(displayError(error))
      return false
    }
  }
  cancelActiveTurn(): void {
    const chat = this.store.getSnapshot().selectedChat
    if (!chat?.activeTurn) return
    try {
      this.requireClient().dispatch(chat.resource, { type: ActionType.ChatTurnCancelled, turnId: chat.activeTurn.id, duration: 0 })
    } catch (error) {
      this.store.setNotice(displayError(error))
    }
  }
  confirmToolCall(chat: URI, turnId: string, toolCallId: string, option: ToolConfirmation): boolean {
    try {
      const submitted = this.toolConfirmations.submit(this.requireClient(), chat, turnId, toolCallId, option)
      if (submitted) this.store.setNotice(undefined)
      return submitted
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
  private isCurrentConnection(connectionGeneration?: number): boolean {
    return connectionGeneration === this.connection.connectionGeneration
  }
}
