import type { RootState, SessionState, URI } from "@microsoft/agent-host-protocol"
import { SessionLifecycle } from "@microsoft/agent-host-protocol"
import { describe, expect, it } from "vitest"

import {
  modelSelection,
  modelSelectionWithThinking,
  reasoningConfig,
} from "./model-config"
import { AhpStore } from "./store"

const session = "ahp-session:/model-test" as URI

function rootState(): RootState {
  return {
    agents: [{
      provider: "openai",
      displayName: "OpenAI",
      description: "Models",
      models: [{
        id: "gpt-example",
        provider: "openai",
        name: "Example model",
        configSchema: {
          type: "object",
          properties: {
            thinkingLevel: {
              type: "string",
              title: "Reasoning level",
              description: "How deeply the model reasons.",
              default: "medium",
              enum: ["low", "medium", "high"],
              enumLabels: ["Quick", "Balanced", "Thorough"],
              enumDescriptions: ["Fast answer", "Balanced answer", "Deep answer"],
            },
          },
        },
      }],
    }],
  }
}

function sessionState(): SessionState {
  return {
    provider: "openai",
    title: "Model session",
    status: 1,
    lifecycle: SessionLifecycle.Ready,
    activeClients: [],
    chats: [],
    config: {
      schema: {
        type: "object",
        properties: {
          model: {
            type: "object",
            title: "Model",
            sessionMutable: true,
            properties: {},
          },
        },
      },
      values: { model: { id: "gpt-example", config: { thinkingLevel: "medium" } } },
    },
  }
}

describe("model configuration projection", () => {
  it("projects and deeply freezes the root catalog and selected session configuration", () => {
    const root = rootState()
    const store = new AhpStore()
    store.applySnapshot({ resource: "ahp-root://", state: root, fromSeq: 1 })
    store.applySnapshot({ resource: session, state: sessionState(), fromSeq: 2 })
    store.setSelection(session)

    const snapshot = store.getSnapshot()
    expect(snapshot.models).toMatchObject([{
      id: "gpt-example",
      name: "Example model",
      configSchema: { properties: { thinkingLevel: { enum: ["low", "medium", "high"] } } },
    }])
    expect(snapshot.selectedSession?.config?.values).toEqual({
      model: { id: "gpt-example", config: { thinkingLevel: "medium" } },
    })
    expect(Object.isFrozen(snapshot.models)).toBe(true)
    expect(Object.isFrozen(snapshot.models[0])).toBe(true)
    expect(Object.isFrozen(snapshot.models[0].configSchema?.properties.thinkingLevel.enum)).toBe(true)
    expect(Object.isFrozen(snapshot.selectedSession?.config?.values.model)).toBe(true)

    root.agents[0].models[0].name = "Mutated host model"
    expect(snapshot.models[0].name).toBe("Example model")
  })

  it("uses labels and descriptions while omitting the model-default thinking value", () => {
    const store = new AhpStore()
    store.applySnapshot({ resource: "ahp-root://", state: rootState(), fromSeq: 1 })
    const reasoning = reasoningConfig(store.getSnapshot().models[0])

    expect(reasoning).toEqual({
      title: "Reasoning level",
      description: "How deeply the model reasons.",
      default: "medium",
      options: [
        { value: "low", label: "Quick", description: "Fast answer" },
        { value: "medium", label: "Balanced", description: "Balanced answer" },
        { value: "high", label: "Thorough", description: "Deep answer" },
      ],
    })
    expect(modelSelection("gpt-example")).toEqual({ id: "gpt-example" })
    expect(modelSelection("gpt-example", "high")).toEqual({
      id: "gpt-example",
      config: { thinkingLevel: "high" },
    })
    const configured = { id: "gpt-example", config: { thinkingLevel: "medium", contextSize: 200_000 } }
    expect(modelSelectionWithThinking(configured, "high")).toEqual({
      id: "gpt-example",
      config: { thinkingLevel: "high", contextSize: 200_000 },
    })
    expect(modelSelectionWithThinking(configured)).toEqual({
      id: "gpt-example",
      config: { contextSize: 200_000 },
    })
  })
})
