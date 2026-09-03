import { renderToStaticMarkup } from "react-dom/server"
import { describe, expect, it } from "vitest"

import type { AppSnapshot } from "@/ahp/store"

import { Composer } from "./composer"
import { SessionSidebar } from "./session-sidebar"

const models = [{ id: "gpt-example", provider: "openai", name: "Example" }]
const snapshot: AppSnapshot = {
  connection: { phase: "connected" },
  models,
  sessions: [],
}

describe("draft UI placement", () => {
  it("renders the model picker in the chat composer, not the session sidebar", () => {
    const sidebar = renderToStaticMarkup(
      <SessionSidebar
        snapshot={snapshot}
        onCreateSession={async () => true}
        onRefresh={async () => {}}
        onSelect={async () => {}}
      />,
    )
    const composer = renderToStaticMarkup(
      <Composer
        chat={{ resource: "ahp-chat:/draft", title: "Draft", turns: [], draft: { text: "", model: models[0] } }}
        connected
        models={models}
        onDraftChange={() => true}
        onSend={() => true}
        onCancel={() => {}}
      />,
    )

    expect(sidebar).not.toContain("<select")
    expect(composer).toContain("<select")
  })
})
