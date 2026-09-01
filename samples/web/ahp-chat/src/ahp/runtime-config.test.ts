import { describe, expect, it } from "vitest"

import { resolveWebSocketUrl, validateWebSocketUrl } from "./runtime-config"

describe("resolveWebSocketUrl", () => {
  it("prefers an address supplied by the static host", () => {
    expect(resolveWebSocketUrl({
      runtimeConfig: { webSocketUrl: "wss://agent.example.test/socket" },
      location: { protocol: "https:", host: "app.example.test" },
      isDevelopment: false,
    })).toBe("wss://agent.example.test/socket")
  })

  it("uses the local sample host during Vite development", () => {
    expect(resolveWebSocketUrl({
      location: { protocol: "http:", host: "localhost:5173" },
      isDevelopment: true,
    })).toBe("ws://127.0.0.1:8787/ahp")
  })

  it.each([
    ["https:", "app.example.test", "wss://app.example.test/ahp"],
    ["http:", "app.example.test:8080", "ws://app.example.test:8080/ahp"],
  ])("derives %s static-host URLs", (protocol, host, expected) => {
    expect(resolveWebSocketUrl({
      location: { protocol, host },
      isDevelopment: false,
    })).toBe(expected)
  })

  it("accepts only WebSocket schemes", () => {
    expect(validateWebSocketUrl("ws://127.0.0.1:8787/ahp")).toBe("ws://127.0.0.1:8787/ahp")
    expect(() => validateWebSocketUrl("https://example.test/ahp")).toThrow("ws:// or wss://")
  })
})
