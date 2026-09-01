export interface AhpSampleConfig {
  webSocketUrl?: string
}

declare global {
  interface Window {
    __AHP_SAMPLE_CONFIG__?: AhpSampleConfig
  }
}

export interface UrlResolverInput {
  runtimeConfig?: AhpSampleConfig
  location: Pick<Location, "host" | "protocol">
  isDevelopment: boolean
}

const developmentUrl = "ws://127.0.0.1:8787/ahp"

export function resolveWebSocketUrl({
  runtimeConfig,
  location,
  isDevelopment,
}: UrlResolverInput): string {
  const configured = runtimeConfig?.webSocketUrl?.trim()
  if (configured) return configured

  if (isDevelopment) return developmentUrl

  const scheme = location.protocol === "https:" ? "wss:" : "ws:"
  return `${scheme}//${location.host}/ahp`
}

export function resolveBrowserWebSocketUrl(): string {
  return resolveWebSocketUrl({
    runtimeConfig: window.__AHP_SAMPLE_CONFIG__,
    location: window.location,
    isDevelopment: import.meta.env.DEV,
  })
}

export function isRuntimeUrlConfigured(): boolean {
  return Boolean(window.__AHP_SAMPLE_CONFIG__?.webSocketUrl?.trim())
}

export function readLastSuccessfulUrl(): string | undefined {
  try {
    return window.localStorage.getItem("ahp-chat.last-websocket-url") ?? undefined
  } catch {
    return undefined
  }
}

export function saveLastSuccessfulUrl(url: string): void {
  try {
    window.localStorage.setItem("ahp-chat.last-websocket-url", url)
  } catch {
    // Storage can be disabled; it is only a convenience.
  }
}

export function initialWebSocketUrl(): string {
  if (!isRuntimeUrlConfigured()) {
    const saved = readLastSuccessfulUrl()
    if (saved) return saved
  }
  return resolveBrowserWebSocketUrl()
}

export function validateWebSocketUrl(value: string): string {
  const url = new URL(value.trim())
  if (url.protocol !== "ws:" && url.protocol !== "wss:") {
    throw new Error("Use a ws:// or wss:// address.")
  }
  return url.toString()
}
