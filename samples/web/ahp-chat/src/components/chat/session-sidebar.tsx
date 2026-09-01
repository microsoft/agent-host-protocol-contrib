import { useEffect, useId, useRef, useState } from "react"
import { LoaderCircle, MessageSquarePlus, RefreshCw } from "lucide-react"

import {
  isAvailableModelSelection,
  loadLastModelSelection,
  modelSelectionFromConfig,
  saveLastModelSelection,
  toModelSelection,
  type ModelSelectionView,
} from "@/ahp/model-config"
import type { ResolveSessionConfigResult } from "@microsoft/agent-host-protocol"
import type { AppSnapshot } from "@/ahp/store"
import { formatWorkspace, loadLastWorkspaceInput } from "@/ahp/workspace"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Separator } from "@/components/ui/separator"
import { cn } from "@/lib/utils"

import { ModelPicker } from "./model-picker"

interface SessionSidebarProps {
  snapshot: AppSnapshot
  onCreateSession: (workspaceInput: string, config: Record<string, unknown>) => Promise<boolean>
  onResolveSessionConfig: (
    workspaceInput: string,
    config: Record<string, unknown>,
  ) => Promise<ResolveSessionConfigResult | undefined>
  onRefresh: () => Promise<void>
  onSelect: (session: string) => Promise<void>
}

export function SessionSidebar({
  snapshot,
  onCreateSession,
  onResolveSessionConfig,
  onRefresh,
  onSelect,
}: SessionSidebarProps) {
  const connected = snapshot.connection.phase === "connected"
  const loading = snapshot.connection.phase === "reconnecting"
  const workspaceId = useId()
  const request = useRef(0)
  const [workspaceInput, setWorkspaceInput] = useState(loadLastWorkspaceInput)
  const [selection, setSelection] = useState<ModelSelectionView | undefined>(loadLastModelSelection)
  const [resolved, setResolved] = useState<ResolveSessionConfigResult>()
  const [resolving, setResolving] = useState(false)
  const [creating, setCreating] = useState(false)
  const hostKey = snapshot.connection.address ?? ""
  const catalogKey = snapshot.models.map((model) => `${model.id}:${JSON.stringify(model.configSchema)}`).join("|")

  function configFor(next: ModelSelectionView | undefined, includeResolved = true): Record<string, unknown> {
    const config = includeResolved ? { ...(resolved?.values ?? {}) } : {}
    if (next) config.model = toModelSelection(next)
    else delete config.model
    return config
  }

  async function resolve(
    next: ModelSelectionView | undefined,
    includeResolved = true,
  ): Promise<ResolveSessionConfigResult | undefined> {
    const token = ++request.current
    setResolving(true)
    const result = await onResolveSessionConfig(workspaceInput, configFor(next, includeResolved))
    if (token !== request.current) return undefined
    setResolving(false)
    if (!result) return undefined
    const resolvedSelection = modelSelectionFromConfig(result.values)
    setResolved(result)
    setSelection(resolvedSelection)
    saveLastModelSelection(isAvailableModelSelection(snapshot.models, resolvedSelection) ? resolvedSelection : undefined)
    return result
  }

  useEffect(() => {
    if (!connected) {
      setResolved(undefined)
      return
    }
    const saved = isAvailableModelSelection(snapshot.models, selection) ? selection : undefined
    void resolve(saved, false)
    // The resolver is deliberately driven by connection/catalog changes; picker changes call it directly.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connected, hostKey, catalogKey])

  async function createSession(): Promise<void> {
    if (creating || resolving) return
    setCreating(true)
    try {
      const config = await resolve(selection)
      if (config) await onCreateSession(workspaceInput, config.values)
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
          <Button size="sm" onClick={() => void createSession()} disabled={!connected || creating || resolving}>
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
          onBlur={() => void resolve(selection)}
          placeholder={"C:\\work\\project, /work/project, or file:///"}
        />
        <ModelPicker
          models={snapshot.models}
          selection={selection}
          disabled={!connected || creating || resolving}
          onChange={(next) => {
            setSelection(next)
            void resolve(next)
          }}
        />
        <p className="text-xs text-muted-foreground">
          {resolving ? "Loading host defaults…" : "Workspace paths are interpreted on the host machine."}
        </p>
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
