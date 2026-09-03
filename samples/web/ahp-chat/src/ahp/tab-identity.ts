import { pageClientId, rotateTabClientId, tabClientId } from "./ids"

const channelName = "ahp-chat.tab-client-id-claim"

type ClaimMessage = {
  type: "probe" | "contend" | "claimed"
  candidate: string
  nonce: string
}

export interface ClaimChannel {
  onmessage: ((event: MessageEvent<ClaimMessage>) => void) | null
  postMessage(message: ClaimMessage): void
  close(): void
}

export interface TabIdentityProvider {
  uniqueTabClientId(): Promise<string>
  dispose(): void
}

export interface TabIdentityOptions {
  candidate?: () => string
  rotateCandidate?: () => string
  pageId?: () => string
  nonce?: string
  waitMs?: number
  createChannel?: () => ClaimChannel | undefined
}

interface ClaimAttempt {
  candidate: string
  contenders: Set<string>
  hasLivePeer: boolean
}

interface Waiter {
  timer: number
  resolve: () => void
}

function createChannel(): ClaimChannel | undefined {
  try {
    return typeof BroadcastChannel === "function"
      ? new BroadcastChannel(channelName) as unknown as ClaimChannel
      : undefined
  } catch {
    return undefined
  }
}

export class TabIdentity implements TabIdentityProvider {
  private readonly candidate: () => string
  private readonly rotateCandidate: () => string
  private readonly pageId: () => string
  private readonly nonce: string
  private readonly waitMs: number
  private readonly openChannel: () => ClaimChannel | undefined
  private readonly waiters = new Set<Waiter>()
  private readonly onPageHide = () => this.dispose()
  private channel?: ClaimChannel
  private attempt?: ClaimAttempt
  private claimedId?: string
  private resolving?: Promise<string>
  private disposed = false
  private listeningForPageHide = false

  constructor(options: TabIdentityOptions = {}) {
    this.candidate = options.candidate ?? tabClientId
    this.rotateCandidate = options.rotateCandidate ?? rotateTabClientId
    this.pageId = options.pageId ?? pageClientId
    this.nonce = options.nonce ?? this.pageId()
    this.waitMs = options.waitMs ?? 35
    this.openChannel = options.createChannel ?? createChannel
  }

  uniqueTabClientId(): Promise<string> {
    if (this.claimedId) return Promise.resolve(this.claimedId)
    this.resolving ??= this.claim()
    return this.resolving
  }

  dispose(): void {
    if (this.disposed) return
    this.disposed = true
    if (this.listeningForPageHide) globalThis.removeEventListener?.("pagehide", this.onPageHide)
    this.listeningForPageHide = false
    this.attempt = undefined
    this.channel?.close()
    this.channel = undefined
    for (const waiter of this.waiters) {
      globalThis.clearTimeout(waiter.timer)
      waiter.resolve()
    }
    this.waiters.clear()
  }

  private async claim(): Promise<string> {
    const channel = this.getChannel()
    if (!channel) {
      this.claimedId = this.pageId()
      return this.claimedId
    }
    try {
      let candidate = this.candidate()
      for (let tries = 0; tries < 3 && !this.disposed; tries += 1) {
        if (!await this.hasCollision(channel, candidate)) {
          this.claimedId = candidate
          return candidate
        }
        candidate = this.rotateCandidate()
      }
    } catch {
      this.channel?.close()
      this.channel = undefined
    }
    this.claimedId = this.pageId()
    return this.claimedId
  }

  private getChannel(): ClaimChannel | undefined {
    if (this.channel) return this.channel
    if (this.disposed) return undefined
    try {
      const channel = this.openChannel()
      if (!channel) return undefined
      channel.onmessage = (event) => this.handleMessage(event.data)
      this.channel = channel
      globalThis.addEventListener?.("pagehide", this.onPageHide, { once: true })
      this.listeningForPageHide = true
      return channel
    } catch {
      return undefined
    }
  }

  private async hasCollision(channel: ClaimChannel, candidate: string): Promise<boolean> {
    const attempt: ClaimAttempt = { candidate, contenders: new Set(), hasLivePeer: false }
    this.attempt = attempt
    channel.postMessage({ type: "probe", candidate, nonce: this.nonce })
    channel.postMessage({ type: "contend", candidate, nonce: this.nonce })
    await this.wait()
    if (this.attempt !== attempt) return true
    this.attempt = undefined
    return attempt.hasLivePeer || [...attempt.contenders, this.nonce].sort()[0] !== this.nonce
  }

  private handleMessage(message: ClaimMessage): void {
    if (!message || message.nonce === this.nonce) return
    if (this.claimedId === message.candidate) {
      if (message.type !== "claimed") this.channel?.postMessage({ type: "claimed", candidate: message.candidate, nonce: this.nonce })
      return
    }
    if (this.attempt?.candidate !== message.candidate) return
    if (message.type === "claimed") this.attempt.hasLivePeer = true
    if (message.type === "probe") this.channel?.postMessage({ type: "contend", candidate: message.candidate, nonce: this.nonce })
    if (message.type === "contend") this.attempt.contenders.add(message.nonce)
  }

  private wait(): Promise<void> {
    return new Promise((resolve) => {
      const waiter: Waiter = { timer: 0, resolve }
      waiter.timer = globalThis.setTimeout(() => {
        this.waiters.delete(waiter)
        resolve()
      }, this.waitMs)
      this.waiters.add(waiter)
    })
  }
}
