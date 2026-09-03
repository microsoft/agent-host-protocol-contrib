import { useId, useState } from "react"
import { LoaderCircle, MessageSquarePlus, RefreshCw } from "lucide-react"

import type { AppSnapshot } from "@/ahp/store"
import { formatWorkspace, loadLastWorkspaceInput } from "@/ahp/workspace"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Separator } from "@/components/ui/separator"
import { cn } from "@/lib/utils"

interface SessionSidebarProps {
  snapshot: AppSnapshot
  onCreateSession: (workspaceInput: string) => Promise<boolean>
  onRefresh: () => Promise<void>
  onSelect: (session: string) => Promise<void>
}

export function SessionSidebar({ snapshot, onCreateSession, onRefresh, onSelect }: SessionSidebarProps) {
  const loading = snapshot.connection.phase === "reconnecting"
  const workspaceId = useId()
  const [workspaceInput, setWorkspaceInput] = useState(loadLastWorkspaceInput)
  const [creating, setCreating] = useState(false)

  async function createSession(): Promise<void> {
    if (creating) return
    setCreating(true)
    try {
      await onCreateSession(workspaceInput)
    } finally {
      setCreating(false)
    }
  }

  return (
    <aside className="flex min-h-0 w-full flex-col border-b bg-muted/20 md:w-72 md:border-r md:border-b-0">
      <div className="flex items-center justify-between gap-2 p-3">
        <span className="text-sm font-medium">Sessions</span>
        <div className="flex gap-1">
          <Button variant="ghost" size="icon-sm" onClick={() => void onRefresh()} aria-label="Refresh sessions">
            <RefreshCw className={loading ? "animate-spin" : ""} />
          </Button>
          <Button size="sm" onClick={() => void createSession()} disabled={creating}>
            <MessageSquarePlus /> New
          </Button>
        </div>
      </div>
      <Separator />
      <form
        className="space-y-2 border-b p-3"
        onSubmit={(event) => {
          event.preventDefault()
          void createSession()
        }}
      >
        <label htmlFor={workspaceId} className="text-xs font-medium">New session workspace (optional)</label>
        <Input
          id={workspaceId}
          value={workspaceInput}
          onChange={(event) => setWorkspaceInput(event.target.value)}
          placeholder={"C:\\work\\project, /work/project, or file:///"}
        />
        <p className="text-xs text-muted-foreground">An absolute path or file URI, interpreted on the host machine.</p>
      </form>
      <nav aria-label="Sessions" className="min-h-0 space-y-1 overflow-y-auto p-2">
        {snapshot.sessions.length === 0 && (
          <p className="p-3 text-sm text-muted-foreground">No sessions yet. Start one to open a chat.</p>
        )}
        {snapshot.sessions.map((session) => {
          const selected = session.resource === snapshot.selectedSession?.resource
          return (
            <button
              key={session.resource}
              type="button"
              aria-current={selected ? "page" : undefined}
              onClick={() => void onSelect(session.resource)}
              className={cn(
                "flex w-full flex-col gap-1 rounded-lg p-2.5 text-left text-sm outline-none transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring",
                selected && "bg-muted",
              )}
            >
              <span className="truncate font-medium">{session.title || "Untitled session"}</span>
              <span className="truncate text-xs text-muted-foreground">{formatWorkspace(session.workingDirectories[0])}</span>
              <span className="truncate text-xs text-muted-foreground">{session.activity ?? session.provider}</span>
            </button>
          )
        })}
        {loading && (
          <div className="flex items-center gap-2 p-3 text-xs text-muted-foreground">
            <LoaderCircle className="size-3 animate-spin" /> Waiting for the host…
          </div>
        )}
      </nav>
    </aside>
  )
}
