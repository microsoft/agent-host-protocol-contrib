import { MessageKind, type Message } from "@microsoft/agent-host-protocol"

import {
  modelSelectionFromConfig,
  toModelSelection,
  type ModelSelectionView,
} from "./model-config"

export interface DraftView {
  text: string
  model?: ModelSelectionView
}

export const draftExpectedTurnIdMeta = "com.microsoft.agent-host-protocol-samples.expectedTurnId"

export function projectDraft(draft?: Message): DraftView | undefined {
  if (!draft) return undefined
  const model = modelSelectionFromConfig({ model: draft.model })
  return {
    text: draft.text,
    ...(model ? { model } : {}),
  }
}

export function draftMessage(draft: DraftView, expectedTurnId?: string | null): Message | undefined {
  if (!draft.text && !draft.model) return undefined
  return {
    text: draft.text,
    origin: { kind: MessageKind.User },
    ...(draft.model ? { model: toModelSelection(draft.model) } : {}),
    ...(expectedTurnId !== undefined
      ? { _meta: { [draftExpectedTurnIdMeta]: expectedTurnId } }
      : {}),
  }
}

export function sameDraft(left?: DraftView, right?: DraftView): boolean {
  return left?.text === right?.text
    && left?.model?.id === right?.model?.id
    && JSON.stringify(Object.entries(left?.model?.config ?? {}).sort()) === JSON.stringify(Object.entries(right?.model?.config ?? {}).sort())
}

export class DraftSynchronizer {
  private value: DraftView
  private dirty = false

  constructor(initial: DraftView) {
    this.value = initial
  }

  get draft(): DraftView {
    return this.value
  }

  local(draft: DraftView): void {
    this.value = draft
    this.dirty = true
  }

  reset(draft: DraftView): void {
    this.value = draft
    this.dirty = false
  }

  applyAuthoritative(draft: DraftView | undefined, fallback: DraftView): void {
    if (sameDraft(draft, this.value)) this.dirty = false
    else if (!this.dirty) this.value = draft ?? fallback
  }
}

export class DraftDebouncer {
  private timer?: number
  private readonly send: (draft: DraftView, expectedTurnId: string | undefined) => boolean
  private readonly delay: number

  constructor(send: (draft: DraftView, expectedTurnId: string | undefined) => boolean, delay = 250) {
    this.send = send
    this.delay = delay
  }

  schedule(draft: DraftView, expectedTurnId: string | undefined): void {
    this.cancel()
    this.timer = globalThis.setTimeout(() => {
      this.timer = undefined
      this.send(draft, expectedTurnId)
    }, this.delay)
  }

  flush(draft: DraftView, expectedTurnId: string | undefined): boolean {
    this.cancel()
    return this.send(draft, expectedTurnId)
  }

  cancel(): void {
    if (this.timer !== undefined) globalThis.clearTimeout(this.timer)
    this.timer = undefined
  }
}
