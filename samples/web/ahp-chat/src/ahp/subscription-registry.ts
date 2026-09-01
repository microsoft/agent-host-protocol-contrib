import type { Snapshot, URI } from "@microsoft/agent-host-protocol"
import { type AhpClient, type Subscription } from "@microsoft/agent-host-protocol/client"

const rootUri = "ahp-root://" as const

interface SubscriptionContext {
  client: AhpClient
  token: number
  isCurrent: () => boolean
  onSnapshot: (snapshot: Snapshot) => void
  onSubscription: (subscription: Subscription, token: number) => void
  onUnsubscribeError: (error: unknown) => void
}

interface PendingSubscription {
  client: AhpClient
  token: number
  task: Promise<void>
}

/**
 * Owns the client-side subscription set separately from connection transport
 * lifetime, so a reconnect can resume only the resources the UI still needs.
 */
export class SubscriptionRegistry {
  private desired = new Set<URI>([rootUri])
  private readonly live = new Map<URI, Subscription>()
  private readonly pending = new Map<URI, PendingSubscription>()

  get desiredUris(): readonly URI[] {
    return [...this.desired]
  }

  reset(): void {
    this.closeConsumers()
    this.pending.clear()
    this.desired = new Set([rootUri])
  }

  attachAll(context: SubscriptionContext): void {
    for (const uri of this.desired) {
      const subscription = context.client.attachSubscription(uri)
      this.live.set(uri, subscription)
      context.onSubscription(subscription, context.token)
    }
  }

  async subscribe(uri: URI, context: SubscriptionContext): Promise<void> {
    if (!context.isCurrent()) return
    this.desired.add(uri)
    if (this.live.has(uri)) {
      const result = await context.client.request("subscribe", { channel: uri })
      if (this.keep(uri, context) && result.snapshot) context.onSnapshot(result.snapshot)
      return
    }

    const existing = this.pending.get(uri)
    if (existing?.client === context.client && existing.token === context.token) {
      await existing.task
      return
    }

    const task = this.openSubscription(uri, context)
    const pending: PendingSubscription = { client: context.client, token: context.token, task }
    this.pending.set(uri, pending)
    try {
      await task
    } finally {
      if (this.pending.get(uri) === pending) this.pending.delete(uri)
    }
  }

  async refresh(uri: URI, context: SubscriptionContext): Promise<void> {
    if (!this.keep(uri, context)) return
    const result = await context.client.request("subscribe", { channel: uri })
    if (this.keep(uri, context) && result.snapshot) context.onSnapshot(result.snapshot)
  }

  async reconcile(session?: URI, chat?: URI, context?: SubscriptionContext): Promise<void> {
    const next = new Set<URI>([rootUri])
    if (session) next.add(session)
    if (chat) next.add(chat)

    const stale = new Set<URI>([...this.desired, ...this.live.keys(), ...this.pending.keys()])
    this.desired = next
    if (!context) {
      this.closeConsumersExcept(next)
      return
    }
    await Promise.all([...stale]
      .filter((uri) => !next.has(uri))
      .map((uri) => this.release(uri, context)))
  }

  removeMissing(uris: readonly URI[]): void {
    for (const uri of uris) {
      if (uri === rootUri) continue
      this.desired.delete(uri)
      const subscription = this.live.get(uri)
      if (subscription) {
        this.live.delete(uri)
        void subscription.close()
      }
    }
  }

  closeConsumers(): void {
    for (const subscription of this.live.values()) void subscription.close()
    this.live.clear()
  }

  private async release(uri: URI, context: SubscriptionContext): Promise<void> {
    if (this.desired.has(uri) || !context.isCurrent()) return
    const subscription = this.live.get(uri)
    if (subscription) {
      this.live.delete(uri)
      await subscription.close()
    }

    if (this.desired.has(uri) || !context.isCurrent()) return
    try {
      await context.client.unsubscribe(uri)
    } catch (error) {
      if (!this.desired.has(uri) && context.isCurrent()) context.onUnsubscribeError(error)
    }
  }

  private async openSubscription(uri: URI, context: SubscriptionContext): Promise<void> {
    const { result, subscription } = await context.client.subscribe(uri)
    if (!this.keep(uri, context)) {
      await subscription.close()
      await this.release(uri, context)
      return
    }
    this.live.set(uri, subscription)
    context.onSubscription(subscription, context.token)
    if (result.snapshot) context.onSnapshot(result.snapshot)
  }

  private keep(uri: URI, context: SubscriptionContext): boolean {
    return this.desired.has(uri) && context.isCurrent()
  }

  private closeConsumersExcept(keep: ReadonlySet<URI>): void {
    for (const [uri, subscription] of this.live) {
      if (keep.has(uri)) continue
      this.live.delete(uri)
      void subscription.close()
    }
  }
}
