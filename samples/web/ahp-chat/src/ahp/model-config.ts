import type {
  ConfigPropertySchema,
  ConfigSchema,
  JsonPrimitive,
  ModelSelection,
  RootState,
  SessionConfigSchema,
  SessionConfigState,
} from "@microsoft/agent-host-protocol"

const lastModelSelectionKey = "ahp-chat.last-model-selection"

export interface ConfigPropertyView {
  type: ConfigPropertySchema["type"]
  title: string
  description?: string
  default?: unknown
  enum?: readonly JsonPrimitive[]
  enumLabels?: readonly string[]
  enumDescriptions?: readonly string[]
  readOnly?: boolean
  sessionMutable?: boolean
  enumDynamic?: boolean
  items?: ConfigPropertyView
  properties?: Readonly<Record<string, ConfigPropertyView>>
  required?: readonly string[]
  additionalProperties?: ConfigPropertyView
}

export interface ConfigSchemaView {
  type: "object"
  properties: Readonly<Record<string, ConfigPropertyView>>
  required?: readonly string[]
}

export interface ModelView {
  id: string
  provider: string
  name: string
  configSchema?: ConfigSchemaView
}

export interface ModelSelectionView {
  id: string
  config?: Readonly<Record<string, JsonPrimitive>>
}

export interface SessionConfigView {
  schema: ConfigSchemaView
  values: Readonly<Record<string, unknown>>
}

export interface ReasoningOption {
  value: JsonPrimitive
  label: string
  description?: string
}

export interface ReasoningConfig {
  title: string
  description?: string
  default?: JsonPrimitive
  options: readonly ReasoningOption[]
}

function copyValue(value: unknown): unknown {
  if (!value || typeof value !== "object") return value
  if (Array.isArray(value)) return value.map(copyValue)
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, copyValue(item)]))
}

function projectProperty(property: ConfigPropertySchema): ConfigPropertyView {
  const sessionMutable = "sessionMutable" in property ? property.sessionMutable : undefined
  const enumDynamic = "enumDynamic" in property ? property.enumDynamic : undefined
  return {
    type: property.type,
    title: property.title,
    ...(property.description !== undefined ? { description: property.description } : {}),
    ...(property.default !== undefined ? { default: copyValue(property.default) } : {}),
    ...(property.enum !== undefined ? { enum: [...property.enum] } : {}),
    ...(property.enumLabels !== undefined ? { enumLabels: [...property.enumLabels] } : {}),
    ...(property.enumDescriptions !== undefined ? { enumDescriptions: [...property.enumDescriptions] } : {}),
    ...(property.readOnly !== undefined ? { readOnly: property.readOnly } : {}),
    ...(typeof sessionMutable === "boolean" ? { sessionMutable } : {}),
    ...(typeof enumDynamic === "boolean" ? { enumDynamic } : {}),
    ...(property.items ? { items: projectProperty(property.items) } : {}),
    ...(property.properties
      ? { properties: Object.fromEntries(Object.entries(property.properties).map(([key, item]) => [key, projectProperty(item)])) }
      : {}),
    ...(property.required ? { required: [...property.required] } : {}),
    ...(property.additionalProperties ? { additionalProperties: projectProperty(property.additionalProperties) } : {}),
  }
}

function projectSchema(schema: ConfigSchema | SessionConfigSchema): ConfigSchemaView {
  return {
    type: schema.type,
    properties: Object.fromEntries(Object.entries(schema.properties).map(([key, property]) => [key, projectProperty(property)])),
    ...(schema.required ? { required: [...schema.required] } : {}),
  }
}

export function projectModels(root: RootState): readonly ModelView[] {
  const models = new Map<string, ModelView>()
  for (const agent of root.agents ?? []) {
    for (const model of agent.models ?? []) {
      models.set(model.id, {
        id: model.id,
        provider: model.provider,
        name: model.name,
        ...(model.configSchema ? { configSchema: projectSchema(model.configSchema) } : {}),
      })
    }
  }
  return [...models.values()]
}

export function projectSessionConfig(config?: SessionConfigState): SessionConfigView | undefined {
  return config && { schema: projectSchema(config.schema), values: copyValue(config.values) as Record<string, unknown> }
}

function isPrimitive(value: unknown): value is JsonPrimitive {
  return value === null || ["string", "number", "boolean"].includes(typeof value)
}

export function modelSelectionFromConfig(values?: Readonly<Record<string, unknown>>): ModelSelectionView | undefined {
  const selection = values?.model
  if (!selection || typeof selection !== "object" || typeof (selection as ModelSelection).id !== "string") return undefined
  const config = Object.fromEntries(
    Object.entries((selection as ModelSelection).config ?? {}).filter(([, value]) => isPrimitive(value)),
  ) as Record<string, JsonPrimitive>
  return { id: (selection as ModelSelection).id, ...(Object.keys(config).length ? { config } : {}) }
}

export function modelSelection(id: string, thinkingLevel?: JsonPrimitive): ModelSelectionView {
  return { id, ...(thinkingLevel === undefined ? {} : { config: { thinkingLevel } }) }
}

export function toModelSelection(selection: ModelSelectionView): ModelSelection {
  return { id: selection.id, ...(selection.config ? { config: { ...selection.config } } : {}) }
}

export function modelById(models: readonly ModelView[], id?: string): ModelView | undefined {
  return models.find((model) => model.id === id)
}

export function isAvailableModelSelection(models: readonly ModelView[], selection?: ModelSelectionView): boolean {
  if (!selection) return false
  const model = modelById(models, selection.id)
  if (!model || !selection.config) return Boolean(model)
  const configuredKeys = Object.keys(selection.config)
  const reasoning = reasoningConfig(model)
  return configuredKeys.length === 1
    && configuredKeys[0] === "thinkingLevel"
    && Boolean(reasoning?.options.some((option) => Object.is(option.value, selection.config?.thinkingLevel)))
}

export function reasoningConfig(model?: ModelView): ReasoningConfig | undefined {
  const property = model?.configSchema?.properties.thinkingLevel
  if (!property?.enum?.length) return undefined
  return {
    title: property.title || "Reasoning level",
    ...(property.description ? { description: property.description } : {}),
    ...(isPrimitive(property.default) ? { default: property.default } : {}),
    options: property.enum.map((value, index) => ({
      value,
      label: property.enumLabels?.[index] ?? String(value),
      ...(property.enumDescriptions?.[index] ? { description: property.enumDescriptions[index] } : {}),
    })),
  }
}

export function loadLastModelSelection(): ModelSelectionView | undefined {
  if (typeof localStorage === "undefined") return undefined
  try {
    return modelSelectionFromConfig({ model: JSON.parse(localStorage.getItem(lastModelSelectionKey) ?? "null") })
  } catch {
    return undefined
  }
}

export function saveLastModelSelection(selection?: ModelSelectionView): void {
  if (typeof localStorage === "undefined") return
  if (!selection) localStorage.removeItem(lastModelSelectionKey)
  else localStorage.setItem(lastModelSelectionKey, JSON.stringify(selection))
}
