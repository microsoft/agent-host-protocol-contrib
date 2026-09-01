import {
  SUPPORTED_PROTOCOL_VERSIONS,
  type Snapshot,
  type URI,
} from "@microsoft/agent-host-protocol"
import { AhpClient, type Subscription } from "@microsoft/agent-host-protocol/client"
import { WebSocketTransport } from "@microsoft/agent-host-protocol/ws"

import { consumeSubscription, type ConnectionCallbacks, type TransportOpener } from "./connection-events"
import { displayError } from "./errors"
import { ConnectionEpoch } from "./lifecycle"
import { SubscriptionRegistry } from "./subscription-registry"

const rootUri = "ahp-root://"

export class ConnectionManager {
  private readonly epoch = new ConnectionEpoch()
  private readonly subscriptions = new SubscriptionRegistry()
  private client?: AhpClient
  private address?: string
  private reconnectTimer?: number
  private reconnectAttempt = 0
  private intentionallyStopped = true
  private reconnecting = false
  private lastSeenServerSeq = 0
  private activeToken = 0
  private readonly clientId: string
  private readonly callbacks: ConnectionCallbacks
  private readonly openTransport: TransportOpener

  constructor(
    clientId: string,
    callbacks: ConnectionCallbacks,
    openTransport: TransportOpener = (url) => WebSocketTransport.connect(url),
  ) {
    this.clientId = clientId
    this.callbacks = callbacks
    this.openTransport = openTransport
  }

  get activeClient(): AhpClient | undefined {
    return this.client
  }

  setLastSeenServerSeq(sequence: number): void {
    this.lastSeenServerSeq = sequence
  }

  async connect(address: string): Promise<boolean> {
    await this.stop(false)
    const token = this.epoch.begin()
    this.activeToken = token
    this.address = address
    this.intentionallyStopped = false
    this.reconnectAttempt = 0
    this.callbacks.onConnection({ phase: "connecting", address })

    try {
      const client = await this.openClient(token)
      if (!client) return false
      this.callbacks.onConnection({ phase: "initializing", address })
      const initialized = await client.initialize({
        clientId: this.clientId,
        protocolVersions: [...SUPPORTED_PROTOCOL_VERSIONS],
        initialSubscriptions: [rootUri],
        locale: navigator.language,
      })
      if (!this.current(token, client)) return false
      for (const snapshot of initialized.snapshots) this.receiveSnapshot(snapshot)
      this.lastSeenServerSeq = Math.max(this.lastSeenServerSeq, initialized.serverSeq)
      this.callbacks.onConnection({ phase: "connected", address, protocolVersion: initialized.protocolVersion })
      return true
    } catch (error) {
      if (this.current(token)) {
        this.callbacks.onConnection({ phase: "error", address, error: displayError(error) })
        this.intentionallyStopped = true
        this.subscriptions.closeConsumers()
        const failedClient = this.client
        this.client = undefined
        await failedClient?.shutdown()
      }
      return false
    }
  }

  async disconnect(): Promise<void> {
    await this.stop(true)
  }

  async subscribe(uri: URI): Promise<void> {
    const client = this.requireClient()
    await this.subscriptions.subscribe(uri, this.subscriptionContext(client))
  }

  async refresh(uri: URI): Promise<void> {
    const client = this.requireClient()
    await this.subscriptions.refresh(uri, this.subscriptionContext(client))
  }

  async reconcileActiveSubscriptions(session?: URI, chat?: URI): Promise<void> {
    const client = this.client
    await this.subscriptions.reconcile(
      session,
      chat,
      client ? this.subscriptionContext(client) : undefined,
    )
  }

  private async openClient(token: number): Promise<AhpClient | undefined> {
    const transport = await this.openTransport(this.address ?? "")
    if (!this.current(token)) {
      await transport.close()
      return undefined
    }
    const client = new AhpClient(transport)
    this.client = client
    this.observeClient(client, token)
    client.connect()
    this.subscriptions.attachAll(this.subscriptionContext(client))
    return client
  }

  private observeClient(client: AhpClient, token: number): void {
    void (async () => {
      for await (const state of client.stateChanges()) {
        if (!this.current(token, client) || state.status !== "closed" || this.intentionallyStopped) continue
        this.client = undefined
        this.subscriptions.closeConsumers()
        this.scheduleReconnect(token)
      }
    })()
  }

  private scheduleReconnect(token: number): void {
    if (this.reconnectTimer || this.reconnecting || !this.current(token)) return
    this.reconnectAttempt += 1
    this.callbacks.onConnection({ phase: "reconnecting", address: this.address, reconnectAttempt: this.reconnectAttempt })
    this.reconnectTimer = globalThis.setTimeout(() => {
      this.reconnectTimer = undefined
      void this.reconnect(token)
    }, Math.min(1_000 * this.reconnectAttempt, 5_000))
  }

  private async reconnect(token: number): Promise<void> {
    if (!this.current(token) || this.intentionallyStopped) return
    this.reconnecting = true
    try {
      const client = await this.openClient(token)
      if (!client) return
      const result = await client.reconnect({
        clientId: this.clientId,
        lastSeenServerSeq: this.lastSeenServerSeq,
        subscriptions: [...this.subscriptions.desiredUris],
      })
      if (!this.current(token, client)) return
      if ("actions" in result) {
        for (const action of result.actions) this.callbacks.onEvent({ type: "action", params: action })
        this.subscriptions.removeMissing(result.missing)
      } else {
        for (const snapshot of result.snapshots) this.receiveSnapshot(snapshot)
      }
      this.reconnectAttempt = 0
      this.callbacks.onConnection({ phase: "connected", address: this.address })
      this.callbacks.onReconnected()
    } catch (error) {
      if (this.current(token)) {
        this.callbacks.onConnection({
          phase: "reconnecting",
          address: this.address,
          error: displayError(error),
          reconnectAttempt: this.reconnectAttempt,
        })
        this.subscriptions.closeConsumers()
        const failedClient = this.client
        this.client = undefined
        await failedClient?.shutdown()
        this.reconnecting = false
        this.scheduleReconnect(token)
      }
    } finally {
      this.reconnecting = false
    }
  }

  private subscriptionContext(client: AhpClient) {
    const token = this.activeToken
    return {
      client,
      token,
      isCurrent: () => this.current(token, client),
      onSnapshot: (snapshot: Snapshot) => this.receiveSnapshot(snapshot),
      onSubscription: (subscription: Subscription, subscriptionToken: number) => this.consume(subscription, subscriptionToken),
      onUnsubscribeError: (error: unknown) => {
        this.callbacks.onConnection({ phase: "reconnecting", address: this.address, error: displayError(error) })
      },
    }
  }

  private consume(subscription: Subscription, token: number): void {
    consumeSubscription(
      subscription,
      () => this.current(token) && !this.intentionallyStopped,
      (event) => this.callbacks.onEvent(event),
      (error) => this.callbacks.onConnection({ phase: "reconnecting", address: this.address, error: displayError(error) }),
    )
  }

  private requireClient(): AhpClient {
    if (!this.client) throw new Error("Not connected to an AHP host.")
    return this.client
  }

  private receiveSnapshot(snapshot: Snapshot): void {
    this.lastSeenServerSeq = Math.max(this.lastSeenServerSeq, snapshot.fromSeq)
    this.callbacks.onSnapshot(snapshot)
  }

  private current(token: number, client?: AhpClient): boolean {
    return this.epoch.isCurrent(token) && (!client || this.client === client)
  }

  private async stop(clearAddress: boolean): Promise<void> {
    this.intentionallyStopped = true
    this.epoch.invalidate()
    if (this.reconnectTimer) globalThis.clearTimeout(this.reconnectTimer)
    this.reconnectTimer = undefined
    this.reconnecting = false
    this.subscriptions.reset()
    const client = this.client
    this.client = undefined
    if (client) await client.shutdown()
    if (clearAddress) this.address = undefined
    this.callbacks.onConnection({ phase: "disconnected" })
  }
}
