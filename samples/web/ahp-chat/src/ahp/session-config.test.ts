import { describe, expect, it, vi } from "vitest"

import { AhpService, type AhpConnection } from "./service"

const selectedModel = { id: "gpt-example", config: { thinkingLevel: "medium" } }

function connection(request = vi.fn(), dispatch = vi.fn()): AhpConnection {
  return {
    activeClient: { request, dispatch } as unknown as AhpConnection["activeClient"],
    connect: vi.fn(async () => true),
    disconnect: vi.fn(async () => {}),
    refresh: vi.fn(async () => {}),
    reconcileActiveSubscriptions: vi.fn(async () => {}),
    setLastSeenServerSeq: vi.fn(),
    subscribe: vi.fn(async () => {}),
  }
}

describe("session model configuration", () => {
  it("uses typed root requests for resolved configuration and model completions", async () => {
    const request = vi.fn(async (method: string) => method === "resolveSessionConfig"
      ? { schema: { type: "object", properties: {} }, values: { model: selectedModel } }
      : { items: [{ value: "gpt-example", label: "Example model" }] })
    const service = new AhpService(() => connection(request))

    await expect(service.resolveSessionConfig("C:\\work\\sample", { model: selectedModel })).resolves.toMatchObject({
      values: { model: selectedModel },
    })
    expect(request).toHaveBeenLastCalledWith("resolveSessionConfig", {
      channel: "ahp-root://",
      provider: "openai",
      workingDirectory: "file:///C:/work/sample",
      config: { model: selectedModel },
    })

    await expect(service.sessionConfigCompletions("C:\\work\\sample", { model: selectedModel }, "model", "example"))
      .resolves.toEqual([{ value: "gpt-example", label: "Example model" }])
    expect(request).toHaveBeenLastCalledWith("sessionConfigCompletions", {
      channel: "ahp-root://",
      provider: "openai",
      workingDirectory: "file:///C:/work/sample",
      config: { model: selectedModel },
      property: "model",
      query: "example",
    })
  })

  it("creates a session without pushing configuration into session scope", async () => {
    const request = vi.fn(async (method: string) => method === "listSessions" ? { items: [] } : undefined)
    const service = new AhpService(() => connection(request))

    await service.createSession("C:\\work\\sample")

    expect(request).toHaveBeenCalledWith("createSession", {
      channel: expect.stringMatching(/^ahp-session:\//),
      provider: "openai",
      workingDirectories: ["file:///C:/work/sample"],
    })
  })

  it("drops an old-host config response after a connection switch", async () => {
    let resolveFirst: ((value: unknown) => void) | undefined
    const first = connection(vi.fn(() => new Promise((resolve) => { resolveFirst = resolve })))
    const second = connection(vi.fn())
    let active = first.activeClient
    const service = new AhpService(() => ({
      ...first,
      get activeClient() { return active },
    }))

    const pending = service.resolveSessionConfig()
    active = second.activeClient
    resolveFirst?.({ schema: { type: "object", properties: {} }, values: { model: selectedModel } })

    await expect(pending).resolves.toBeUndefined()
    expect(service.getSnapshot().notice).toBeUndefined()
  })
})
