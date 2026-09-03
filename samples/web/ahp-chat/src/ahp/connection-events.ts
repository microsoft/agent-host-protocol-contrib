import type { Snapshot } from "@microsoft/agent-host-protocol"
import type { AhpTransport, Subscription, SubscriptionEvent } from "@microsoft/agent-host-protocol/client"

import type { ConnectionView } from "./store"

export interface ConnectionCallbacks {
  onConnection: (connection: ConnectionView) => void
  onSnapshot: (snapshot: Snapshot) => void
  onEvent: (event: SubscriptionEvent, connectionGeneration?: number) => void
  onReconnected: () => void
}

export type TransportOpener = (url: string) => Promise<AhpTransport>

export function consumeSubscription(
  subscription: Subscription,
  isCurrent: () => boolean,
  onEvent: (event: SubscriptionEvent) => void,
  onError: (error: unknown) => void,
): void {
  void (async () => {
    try {
      for await (const event of subscription) {
        if (!isCurrent()) return
        onEvent(event)
      }
    } catch (error) {
      if (isCurrent()) onError(error)
    }
  })()
}
