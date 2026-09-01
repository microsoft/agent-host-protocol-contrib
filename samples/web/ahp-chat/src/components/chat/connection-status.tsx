import { CircleAlert, LoaderCircle, Radio } from "lucide-react"

import type { ConnectionView } from "@/ahp/store"
import { Badge } from "@/components/ui/badge"

interface ConnectionStatusProps {
  connection: ConnectionView
}

export function ConnectionStatus({ connection }: ConnectionStatusProps) {
  const loading = connection.phase === "connecting" || connection.phase === "initializing" || connection.phase === "reconnecting"
  const label = connection.phase === "reconnecting"
    ? `Reconnecting${connection.reconnectAttempt ? ` (${connection.reconnectAttempt})` : ""}`
    : connection.phase

  return (
    <Badge variant={connection.phase === "error" ? "destructive" : loading ? "secondary" : "outline"}>
      {connection.phase === "error" ? <CircleAlert /> : loading ? <LoaderCircle className="animate-spin" /> : <Radio />}
      {label}
    </Badge>
  )
}
