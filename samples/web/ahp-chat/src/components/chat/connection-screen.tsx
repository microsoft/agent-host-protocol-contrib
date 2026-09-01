import { useState, type FormEvent } from "react"
import { CircleAlert, PlugZap } from "lucide-react"

import type { ConnectionView } from "@/ahp/store"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"

interface ConnectionScreenProps {
  address: string
  connection: ConnectionView
  onConnect: (address: string) => Promise<void>
}

export function ConnectionScreen({ address, connection, onConnect }: ConnectionScreenProps) {
  const [value, setValue] = useState(address)
  const connecting = connection.phase === "connecting" || connection.phase === "initializing"

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await onConnect(value)
  }

  return (
    <main className="grid min-h-svh place-items-center bg-muted/30 px-4 py-10">
      <Card className="w-full max-w-xl shadow-sm">
        <CardHeader>
          <div className="mb-2 flex size-10 items-center justify-center rounded-xl bg-primary text-primary-foreground">
            <PlugZap aria-hidden="true" />
          </div>
          <CardTitle className="text-xl">Connect to an Agent Host</CardTitle>
          <CardDescription>
            This browser connects directly to an Agent Host Protocol WebSocket. Nothing is sent until you connect.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form className="space-y-4" onSubmit={submit}>
            <div className="space-y-2">
              <label className="text-sm font-medium" htmlFor="ahp-websocket-url">WebSocket address</label>
              <Input
                id="ahp-websocket-url"
                value={value}
                onChange={(event) => setValue(event.target.value)}
                inputMode="url"
                autoComplete="url"
                spellCheck={false}
                placeholder="ws://127.0.0.1:8787/ahp"
                disabled={connecting}
                required
              />
            </div>
            {connection.error && (
              <Alert variant="destructive">
                <CircleAlert />
                <AlertTitle>Could not connect</AlertTitle>
                <AlertDescription>{connection.error}</AlertDescription>
              </Alert>
            )}
            <Button className="w-full" type="submit" disabled={connecting}>
              <PlugZap />
              {connecting ? "Connecting…" : "Connect"}
            </Button>
          </form>
          <p className="mt-5 text-xs leading-relaxed text-muted-foreground">
            Development defaults to <code>ws://127.0.0.1:8787/ahp</code>. A static host can set
            <code> window.__AHP_SAMPLE_CONFIG__</code> before this app loads.
          </p>
        </CardContent>
      </Card>
    </main>
  )
}
