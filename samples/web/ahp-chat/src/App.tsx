import { useEffect, useMemo, useState, useSyncExternalStore } from "react"

import { initialWebSocketUrl } from "@/ahp/runtime-config"
import { AhpService } from "@/ahp/service"
import { ChatWorkspace } from "@/components/chat/chat-workspace"
import { ConnectionScreen } from "@/components/chat/connection-screen"

function App() {
  const service = useMemo(() => new AhpService(), [])
  const snapshot = useSyncExternalStore(service.subscribe, service.getSnapshot, service.getSnapshot)
  const [address, setAddress] = useState(initialWebSocketUrl)

  useEffect(() => () => {
    void service.disconnect()
  }, [service])

  const showConnection = snapshot.connection.phase === "disconnected"
    || snapshot.connection.phase === "error"
    || snapshot.connection.phase === "connecting"
    || snapshot.connection.phase === "initializing"

  if (showConnection) {
    return (
      <ConnectionScreen
        address={address}
        connection={snapshot.connection}
        onConnect={async (nextAddress) => {
          setAddress(nextAddress)
          await service.connect(nextAddress)
        }}
      />
    )
  }

  return <ChatWorkspace service={service} snapshot={snapshot} />
}

export default App
