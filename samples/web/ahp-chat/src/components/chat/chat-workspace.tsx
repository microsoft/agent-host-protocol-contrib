import { CircleAlert, MessageSquarePlus, PlugZap, Unplug } from "lucide-react"

import { modelSelectionFromConfig } from "@/ahp/model-config"
import type { AhpService } from "@/ahp/service"
import type { AppSnapshot } from "@/ahp/store"
import { formatWorkspace } from "@/ahp/workspace"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"

import { Composer } from "./composer"
import { ConnectionStatus } from "./connection-status"
import { Conversation } from "./conversation"
import { SessionSidebar } from "./session-sidebar"

interface ChatWorkspaceProps {
  service: AhpService
  snapshot: AppSnapshot
}

export function ChatWorkspace({ service, snapshot }: ChatWorkspaceProps) {
  const connected = snapshot.connection.phase === "connected"
  const session = snapshot.selectedSession

  return (
    <main className="flex min-h-svh flex-col bg-background md:h-svh">
      <header className="flex flex-wrap items-center justify-between gap-3 border-b px-4 py-3 sm:px-6">
        <div className="flex items-center gap-3">
          <div className="grid size-8 place-items-center rounded-lg bg-primary text-primary-foreground"><PlugZap /></div>
          <div>
            <h1 className="font-heading text-base font-semibold">AHP Chat</h1>
            <p className="max-w-72 truncate text-xs text-muted-foreground">{snapshot.connection.address}</p>
          </div>
          <ConnectionStatus connection={snapshot.connection} />
        </div>
        <Button variant="outline" onClick={() => void service.disconnect()}>
          <Unplug /> Change host
        </Button>
      </header>

      {snapshot.notice && (
        <div className="border-b px-4 py-2 sm:px-6">
          <Alert variant="destructive">
            <CircleAlert />
            <AlertTitle>Host response</AlertTitle>
            <AlertDescription>{snapshot.notice}</AlertDescription>
          </Alert>
        </div>
      )}

      <div className="flex min-h-0 flex-1 flex-col md:flex-row">
        <SessionSidebar
          snapshot={snapshot}
          onCreateSession={(workspaceInput, config) => service.createSession(workspaceInput, config)}
          onResolveSessionConfig={service.resolveSessionConfig}
          onRefresh={() => service.refreshSessions()}
          onSelect={(sessionUri) => service.selectSession(sessionUri)}
        />
        <section className="flex min-h-0 flex-1 flex-col">
          {session && (
            <div className="flex flex-wrap items-center justify-between gap-3 border-b px-4 py-3 sm:px-6">
              <div>
                <h2 className="font-medium">{session.title || "Untitled session"}</h2>
                <p className="text-xs text-muted-foreground">Session is {session.lifecycle}</p>
                <p className="max-w-lg truncate text-xs text-muted-foreground">
                  Workspace: {formatWorkspace(session.workingDirectories[0])}
                </p>
                {session.serverTools.length > 0 && (
                  <p className="max-w-lg truncate text-xs text-muted-foreground">
                    Server tools: {session.serverTools.join(", ")}
                  </p>
                )}
              </div>
              <Button variant="outline" size="sm" onClick={() => void service.createChat()} disabled={!connected}>
                <MessageSquarePlus /> New chat
              </Button>
            </div>
          )}
          {session && session.chats.length > 1 && (
            <div className="flex gap-1 overflow-x-auto px-4 py-2 sm:px-6" aria-label="Chats">
              {session.chats.map((chat) => (
                <Button
                  key={chat.resource}
                  variant={chat.resource === snapshot.selectedChat?.resource ? "secondary" : "ghost"}
                  size="sm"
                  onClick={() => void service.selectChat(session.resource, chat.resource)}
                >
                  {chat.title || "Untitled chat"}
                </Button>
              ))}
            </div>
          )}
          <Separator />
          {snapshot.selectedChat ? (
            <Conversation
              chat={snapshot.selectedChat}
              workspace={session?.workingDirectories[0]}
              onConfirmToolCall={(turnId, toolCallId, option) =>
                service.confirmToolCall(snapshot.selectedChat!.resource, turnId, toolCallId, option)}
            />
          ) : (
            <div className="grid min-h-64 flex-1 place-items-center p-6 text-center">
              <div className="max-w-sm space-y-2">
                <h2 className="font-medium">Choose or create a session</h2>
                <p className="text-sm text-muted-foreground">
                  Session and chat snapshots are loaded only after you select them.
                </p>
              </div>
            </div>
          )}
          <Composer
            chat={snapshot.selectedChat}
            connected={connected}
            models={snapshot.models}
            selection={modelSelectionFromConfig(session?.config?.values)}
            configPending={snapshot.pendingSessionConfig === session?.resource}
            onSend={(text) => service.sendPrompt(text)}
            onCancel={() => service.cancelActiveTurn()}
            onChangeModel={(model) => service.changeSessionModel(model)}
          />
        </section>
      </div>
    </main>
  )
}
