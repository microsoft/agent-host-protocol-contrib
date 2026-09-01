import { useState, type KeyboardEvent } from "react"
import { Send, Square } from "lucide-react"

import type { ChatView } from "@/ahp/projection"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"

interface ComposerProps {
  chat?: ChatView
  connected: boolean
  onSend: (text: string) => void
  onCancel: () => void
}

export function Composer({ chat, connected, onSend, onCancel }: ComposerProps) {
  const [value, setValue] = useState("")
  const canSend = Boolean(connected && chat && !chat.activeTurn && value.trim())

  function send(): void {
    if (!canSend) return
    onSend(value)
    setValue("")
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>): void {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault()
      send()
    }
  }

  return (
    <form
      className="border-t bg-background p-3 sm:px-6"
      onSubmit={(event) => {
        event.preventDefault()
        send()
      }}
    >
      <div className="flex items-end gap-2">
        <Textarea
          value={value}
          onChange={(event) => setValue(event.target.value)}
          onKeyDown={onKeyDown}
          placeholder={chat ? "Message the agent…" : "Select a chat to begin"}
          aria-label="Message the agent"
          disabled={!connected || !chat || Boolean(chat.activeTurn)}
          className="min-h-20 resize-none"
        />
        {chat?.activeTurn ? (
          <Button type="button" variant="destructive" onClick={onCancel} aria-label="Cancel active turn">
            <Square /> Cancel
          </Button>
        ) : (
          <Button type="submit" disabled={!canSend} aria-label="Send message">
            <Send /> Send
          </Button>
        )}
      </div>
      <p className="mt-2 text-xs text-muted-foreground">Enter to send · Shift+Enter for a new line</p>
    </form>
  )
}
