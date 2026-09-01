import type { URI } from "@microsoft/agent-host-protocol"

const lastWorkspaceInputKey = "ahp-chat.last-workspace-input"

function encodePath(path: string): string {
  return path.split("/").map(encodeURIComponent).join("/")
}

/**
 * Converts a user-entered absolute local path to a file URI. This only
 * normalizes the value; the connected host remains responsible for validating it.
 */
export function workspaceInputToFileUri(input: string): URI {
  const value = input.trim()
  if (!value) throw new Error("Enter an absolute workspace path or file URI.")

  if (/^file:/i.test(value)) {
    const uri = new URL(value)
    if (uri.protocol !== "file:") throw new Error("Workspace must use the file: URI scheme.")
    return uri.href
  }

  const windowsDrive = /^([A-Za-z]):[\\/](.*)$/.exec(value)
  if (windowsDrive) {
    return new URL(`file:///${windowsDrive[1].toUpperCase()}:/${encodePath(windowsDrive[2].replace(/\\/g, "/"))}`).href
  }

  const windowsUnc = /^\\\\([^\\/]+)[\\/](.+)$/.exec(value)
  if (windowsUnc) {
    return new URL(`file://${windowsUnc[1]}/${encodePath(windowsUnc[2].replace(/\\/g, "/"))}`).href
  }

  if (value.startsWith("/")) return new URL(`file://${encodePath(value)}`).href
  throw new Error("Workspace paths must be absolute (for example C:\\work or /work).")
}

export function formatWorkspace(uri?: URI): string {
  if (!uri) return "Host default workspace"
  try {
    const parsed = new URL(uri)
    if (parsed.protocol !== "file:") return uri
    const path = decodeURIComponent(parsed.pathname)
    if (parsed.host) return `\\\\${parsed.host}${path.replace(/\//g, "\\")}`
    if (/^\/[A-Za-z]:\//.test(path)) return path.slice(1).replace(/\//g, "\\")
    return path
  } catch {
    return uri
  }
}

export function loadLastWorkspaceInput(): string {
  if (typeof localStorage === "undefined") return ""
  return localStorage.getItem(lastWorkspaceInputKey) ?? ""
}

export function saveLastWorkspaceInput(input: string): void {
  if (typeof localStorage === "undefined" || !input.trim()) return
  localStorage.setItem(lastWorkspaceInputKey, input)
}
