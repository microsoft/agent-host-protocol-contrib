const clientIdKey = "ahp-chat.client-id"

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

export function persistentClientId(): string {
  try {
    const existing = window.localStorage.getItem(clientIdKey)
    if (existing) return existing
    const id = `ahp-chat-${uuid()}`
    window.localStorage.setItem(clientIdKey, id)
    return id
  } catch {
    return `ahp-chat-${uuid()}`
  }
}
