import { useEffect, useRef, useState, type KeyboardEvent } from "react"
import { Send, Square } from "lucide-react"

import {
  DraftDebouncer,
  DraftSynchronizer,
  type DraftView,
} from "@/ahp/drafts"
import {
  isAvailableModelSelection,
  loadLastModelSelection,
  saveLastModelSelection,
  type ModelSelectionView,
  type ModelView,
} from "@/ahp/model-config"
import { latestTurnId, type ChatView } from "@/ahp/projection"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"

import { ModelPicker } from "./model-picker"

interface ComposerProps {
  chat?: ChatView
  connected: boolean
  models: readonly ModelView[]
  sessionModel?: ModelSelectionView
  onDraftChange: (draft: DraftView | undefined, expectedTurnId: string | undefined) => boolean
  onSend: (draft: DraftView) => boolean
  onCancel: () => void
}

function initialDraft(chat: ChatView | undefined, sessionModel: ModelSelectionView | undefined, models: readonly ModelView[]): DraftView {
  const remembered = loadLastModelSelection()
  const model = chat?.draft?.model ?? sessionModel
    ?? (isAvailableModelSelection(models, remembered) ? remembered : undefined)
  return { text: chat?.draft?.text ?? "", ...(model ? { model } : {}) }
}

export function Composer({ chat, connected, models, sessionModel, onDraftChange, onSend, onCancel }: ComposerProps) {
  const synchronizer = useRef<DraftSynchronizer | undefined>(undefined)
  if (!synchronizer.current) synchronizer.current = new DraftSynchronizer(initialDraft(chat, sessionModel, models))
  const [draft, setDraft] = useState(() => synchronizer.current!.draft)
  const sync = useRef(onDraftChange)
  const debouncer = useRef<DraftDebouncer | undefined>(undefined)
  if (!debouncer.current) {
    debouncer.current = new DraftDebouncer((next, expectedTurnId) => sync.current(next, expectedTurnId))
  }
  const modelReady = isAvailableModelSelection(models, draft.model)
  const canSend = Boolean(connected && chat && !chat.activeTurn && modelReady && draft.text.trim())

  function update(next: DraftView): void {
    synchronizer.current!.local(next)
    setDraft(next)
    if (isAvailableModelSelection(models, next.model)) saveLastModelSelection(next.model)
    if (connected && chat && !chat.activeTurn) debouncer.current?.schedule(next, latestTurnId(chat))
  }

  function send(): void {
    const next = synchronizer.current!.draft
    if (!canSend || !debouncer.current?.flush(next, latestTurnId(chat))) return
    onSend(next)
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>): void {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault()
      send()
    }
  }

  useEffect(() => {
    sync.current = onDraftChange
  }, [onDraftChange])

  useEffect(() => {
    debouncer.current?.cancel()
    synchronizer.current!.reset(initialDraft(chat, sessionModel, models))
    setDraft(synchronizer.current!.draft)
    // Chat selection and connection ownership intentionally reset local input.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chat?.resource, connected])

  useEffect(() => {
    if (!chat) return
    if (chat.activeTurn && !chat.draft) {
      debouncer.current?.cancel()
      synchronizer.current!.reset(initialDraft(chat, sessionModel, models))
    } else {
      synchronizer.current!.applyAuthoritative(chat.draft, initialDraft(chat, sessionModel, models))
    }
    setDraft(synchronizer.current!.draft)
  }, [chat, sessionModel, models])

  useEffect(() => () => debouncer.current?.cancel(), [])

  return (
    <form
      className="border-t bg-background p-3 sm:px-6"
      onSubmit={(event) => {
        event.preventDefault()
        send()
      }}
    >
      {chat && (
        <div className="mb-3">
          <ModelPicker
            models={models}
            selection={draft.model}
            allowHostDefault={false}
            disabled={!connected || Boolean(chat.activeTurn)}
            onChange={(model) => {
              const next = { text: synchronizer.current!.draft.text }
              update(model ? { ...next, model } : next)
            }}
          />
          {!modelReady && <p role="status" className="mt-1 text-xs text-muted-foreground">Choose a model to continue.</p>}
        </div>
      )}
      <div className="flex items-end gap-2">
        <Textarea
          value={draft.text}
          onChange={(event) => update({ ...synchronizer.current!.draft, text: event.target.value })}
          onKeyDown={onKeyDown}
          placeholder={chat ? "Message the agent…" : "Select a chat to begin"}
          aria-label="Message the agent"
          disabled={!connected || !chat || Boolean(chat.activeTurn) || !modelReady}
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
