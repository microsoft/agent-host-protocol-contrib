import { RpcError, RpcTimeoutError, TransportError } from "@microsoft/agent-host-protocol/client"

export function displayError(error: unknown): string {
  if (error instanceof RpcError) return `Host error ${error.code}: ${error.message.replace(/^RPC error \d+: /, "")}`
  if (error instanceof RpcTimeoutError) return `The ${error.method} request timed out.`
  if (error instanceof TransportError) return `Socket ${error.kind} error: ${error.message}`
  if (error instanceof Error) return error.message
  return "An unexpected error occurred."
}
