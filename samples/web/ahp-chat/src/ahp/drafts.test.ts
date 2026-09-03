import { MessageKind } from "@microsoft/agent-host-protocol"
import { describe, expect, it, vi } from "vitest"

import {
  DraftDebouncer,
  DraftSynchronizer,
  draftMessage,
  projectDraft,
  type DraftView,
} from "./drafts"

const model = { id: "gpt-example", config: { thinkingLevel: "medium", contextSize: 200_000 } }
const remote: DraftView = { text: "Remote draft", model }
const fallback: DraftView = { text: "", model: { id: "gpt-fallback" } }

describe("chat drafts", () => {
  it("projects wire drafts and preserves primitive model config", () => {
    expect(projectDraft({
      text: remote.text,
      origin: { kind: MessageKind.User },
      model,
    })).toEqual(remote)
    expect(draftMessage(remote)).toEqual({
      text: remote.text,
      origin: { kind: MessageKind.User },
      model,
    })
    expect(draftMessage({ text: "" })).toBeUndefined()
  })

  it("accepts remote drafts without newer local input and restores them on reconnect", () => {
    const state = new DraftSynchronizer(fallback)
    state.applyAuthoritative(remote, fallback)
    expect(state.draft).toEqual(remote)

    const local = { text: "Newer local draft", model }
    state.local(local)
    state.applyAuthoritative(remote, fallback)
    expect(state.draft).toEqual(local)

    state.reset(remote)
    expect(state.draft).toEqual(remote)
    state.reset(fallback)
    expect(state.draft).toEqual(fallback)
  })

  it("keeps a rejected turn draft until the host authoritatively clears it", () => {
    const state = new DraftSynchronizer(remote)
    state.local({ text: "Retain me", model })
    expect(state.draft.text).toBe("Retain me")
    state.reset(fallback)
    expect(state.draft).toEqual(fallback)
  })

  it("debounces draft dispatches and flushes the latest input", () => {
    vi.useFakeTimers()
    const send = vi.fn(() => true)
    const drafts = new DraftDebouncer(send, 250)
    drafts.schedule({ text: "a", model }, undefined)
    drafts.schedule({ text: "ab", model }, undefined)
    vi.advanceTimersByTime(249)
    expect(send).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1)
    expect(send).toHaveBeenCalledExactlyOnceWith({ text: "ab", model }, undefined)

    drafts.schedule({ text: "abc", model }, undefined)
    expect(drafts.flush({ text: "abcd", model }, undefined)).toBe(true)
    vi.runAllTimers()
    expect(send).toHaveBeenLastCalledWith({ text: "abcd", model }, undefined)

    drafts.schedule({ text: "stale", model }, undefined)
    drafts.cancel()
    vi.runAllTimers()
    expect(send).toHaveBeenCalledTimes(2)
    vi.useRealTimers()
  })
})
