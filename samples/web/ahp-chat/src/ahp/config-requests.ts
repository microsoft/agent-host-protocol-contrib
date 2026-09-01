import type {
  ResolveSessionConfigResult,
  SessionConfigValueItem,
} from "@microsoft/agent-host-protocol"
import type { AhpClient } from "@microsoft/agent-host-protocol/client"

import { displayError } from "./errors"
import { ConnectionEpoch } from "./lifecycle"
import { AhpStore } from "./store"
import { workspaceInputToFileUri } from "./workspace"

type ConfigClient = Pick<AhpClient, "request">

export class SessionConfigRequests {
  private readonly epoch = new ConnectionEpoch()
  private readonly getClient: () => ConfigClient | undefined
  private readonly store: AhpStore

  constructor(getClient: () => ConfigClient | undefined, store: AhpStore) {
    this.getClient = getClient
    this.store = store
  }

  invalidate(): void {
    this.epoch.invalidate()
  }

  async resolve(
    workspaceInput = "",
    config: Record<string, unknown> = {},
  ): Promise<ResolveSessionConfigResult | undefined> {
    const token = this.epoch.begin()
    const client = this.getClient()
    if (!client) return undefined
    try {
      const workingDirectory = workspaceInput.trim() ? workspaceInputToFileUri(workspaceInput) : undefined
      const result = await client.request("resolveSessionConfig", {
        channel: this.store.rootChannel,
        provider: "openai",
        ...(workingDirectory ? { workingDirectory } : {}),
        ...(Object.keys(config).length ? { config } : {}),
      })
      if (!this.epoch.isCurrent(token) || this.getClient() !== client) return undefined
      this.store.setNotice(undefined)
      return result
    } catch (error) {
      if (this.epoch.isCurrent(token) && this.getClient() === client) this.store.setNotice(displayError(error))
      return undefined
    }
  }

  async completions(
    workspaceInput: string,
    config: Record<string, unknown>,
    property: string,
    query?: string,
  ): Promise<readonly SessionConfigValueItem[]> {
    const client = this.getClient()
    if (!client) return []
    try {
      const workingDirectory = workspaceInput.trim() ? workspaceInputToFileUri(workspaceInput) : undefined
      const result = await client.request("sessionConfigCompletions", {
        channel: this.store.rootChannel,
        provider: "openai",
        ...(workingDirectory ? { workingDirectory } : {}),
        ...(Object.keys(config).length ? { config } : {}),
        property,
        ...(query ? { query } : {}),
      })
      return this.getClient() === client ? result.items : []
    } catch (error) {
      if (this.getClient() === client) this.store.setNotice(displayError(error))
      return []
    }
  }
}
