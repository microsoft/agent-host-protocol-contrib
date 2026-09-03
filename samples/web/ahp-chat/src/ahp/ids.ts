const tabClientIdKey = "ahp-chat.tab-client-id"
let inMemoryTabClientId: string | undefined

function uuid(): string {
  if (typeof crypto.randomUUID === "function") return crypto.randomUUID()
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`
}

export function resourceUri(kind: "session" | "chat"): string {
  return `ahp-${kind}:/${uuid()}`
}

export function turnId(): string {
  return `turn-${uuid()}`
}

function nextClientId(): string {
  return `ahp-chat-${uuid()}`
}

export function pageClientId(): string {
  inMemoryTabClientId ??= nextClientId()
  return inMemoryTabClientId
}

export function tabClientId(): string {
  try {
    const existing = window.sessionStorage.getItem(tabClientIdKey)
    if (existing) return existing
    const id = nextClientId()
    window.sessionStorage.setItem(tabClientIdKey, id)
    return id
  } catch {
    return pageClientId()
  }
}

export function rotateTabClientId(): string {
  try {
    const id = nextClientId()
    window.sessionStorage.setItem(tabClientIdKey, id)
    return id
  } catch {
    return pageClientId()
  }
}
