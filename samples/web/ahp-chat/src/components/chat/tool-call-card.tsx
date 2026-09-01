import { useState } from "react"
import { ShieldCheck, Terminal, TriangleAlert, Wrench } from "lucide-react"

import {
  ConfirmationOptionKind,
  ToolCallConfirmationReason,
  ToolCallStatus,
} from "@microsoft/agent-host-protocol"

import type { ToolCallView, ToolConfirmationOptionView } from "@/ahp/projection-types"
import { formatWorkspace } from "@/ahp/workspace"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"

interface ToolCallCardProps {
  tool: ToolCallView
  workspace?: string
  onConfirm: (option: ToolConfirmationOptionView) => boolean
}

function statusLabel(tool: ToolCallView): string {
  if (tool.status === ToolCallStatus.Completed) return tool.resultSuccess === false ? "Failed" : "Completed"
  if (tool.status === ToolCallStatus.PendingConfirmation) return "Needs approval"
  if (tool.status === ToolCallStatus.Running) return "Running"
  if (tool.status === ToolCallStatus.Cancelled) return "Cancelled"
  return "Preparing"
}

function statusClass(tool: ToolCallView): string {
  if (tool.status === ToolCallStatus.Completed && tool.resultSuccess === false) return "border-destructive text-destructive"
  if (tool.status === ToolCallStatus.PendingConfirmation) return "border-amber-500 text-amber-700 dark:text-amber-400"
  return ""
}

function optionLabel(option: ToolConfirmationOptionView): string {
  if (option.label) return option.label
  return option.id === "always-allow-command" ? "Always allow this command" : option.id
}

export function ToolCallCard({ tool, workspace, onConfirm }: ToolCallCardProps) {
  const [submitted, setSubmitted] = useState(false)
  const persistentOption = tool.options.find((option) => option.id === "always-allow-command")
  const pendingShellConfirmation = tool.name === "run_shell"
    && tool.status === ToolCallStatus.PendingConfirmation
  const disabled = tool.status !== ToolCallStatus.PendingConfirmation || submitted

  return (
    <section className="space-y-3 rounded-lg border bg-muted/30 p-3" aria-label={`${tool.displayName} tool call`}>
      <div className="flex flex-wrap items-center gap-2">
        {tool.name === "run_shell" ? <Terminal className="size-4" /> : <Wrench className="size-4" />}
        <span className="font-medium">{tool.displayName || tool.name}</span>
        <Badge variant="outline" className={statusClass(tool)}>{statusLabel(tool)}</Badge>
      </div>
      {tool.intention && <p className="text-sm text-muted-foreground">{tool.intention}</p>}
      {tool.input && (
        <pre className="max-h-40 overflow-auto rounded-md bg-background p-2 text-xs whitespace-pre-wrap break-words">
          {tool.input}
        </pre>
      )}
      {pendingShellConfirmation && (
        <div className="space-y-2 rounded-md border border-amber-500/50 bg-amber-500/10 p-2 text-sm">
          <p className="font-medium">{tool.confirmationTitle ?? "Allow shell command?"}</p>
          <p>Workspace: <code>{formatWorkspace(workspace)}</code></p>
          <p className="flex gap-2 text-muted-foreground">
            <TriangleAlert className="mt-0.5 size-4 shrink-0" />
            This command runs on the host machine. Its working directory is not a sandbox.
          </p>
          <div className="flex flex-wrap gap-2">
            {tool.options.map((option) => (
              <Button
                key={option.id}
                type="button"
                size="sm"
                variant={option.kind === ConfirmationOptionKind.Deny ? "outline" : "default"}
                disabled={disabled}
                onClick={() => {
                  if (onConfirm(option)) setSubmitted(true)
                }}
              >
                {optionLabel(option)}
              </Button>
            ))}
          </div>
          {persistentOption && (
            <p className="text-xs text-muted-foreground">
              {`"${optionLabel(persistentOption)}" persists only this exact command string, not every invocation of its executable.`}
            </p>
          )}
        </div>
      )}
      {tool.confirmed === ToolCallConfirmationReason.Setting && (
        <p className="flex items-center gap-2 text-sm text-emerald-700 dark:text-emerald-400">
          <ShieldCheck className="size-4" />
          {`${tool.selectedOptionLabel || "Always allow this command"} is saved only for this exact command string.`}
        </p>
      )}
      {tool.resultMessage && <p className="text-sm">{tool.resultMessage}</p>}
      {tool.resultContent.map((result, index) => (
        <pre key={`${result.type}-${index}`} className="max-h-40 overflow-auto rounded-md bg-background p-2 text-xs whitespace-pre-wrap break-words">
          {result.preview}
        </pre>
      ))}
      {tool.resultError && <p className="text-sm text-destructive">{tool.resultError}</p>}
    </section>
  )
}
