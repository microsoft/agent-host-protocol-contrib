import { useId } from "react"

import {
  modelById,
  modelSelection,
  modelSelectionWithThinking,
  reasoningConfig,
  type ModelSelectionView,
  type ModelView,
} from "@/ahp/model-config"

interface ModelPickerProps {
  models: readonly ModelView[]
  selection?: ModelSelectionView
  disabled?: boolean
  allowHostDefault?: boolean
  onChange: (selection: ModelSelectionView | undefined) => void
}

function encoded(value: unknown): string {
  return JSON.stringify(value)
}

export function ModelPicker({
  models,
  selection,
  disabled = false,
  allowHostDefault = true,
  onChange,
}: ModelPickerProps) {
  const modelId = useId()
  const reasoningId = useId()
  const selectedModel = modelById(models, selection?.id)
  const reasoning = reasoningConfig(selectedModel)
  const currentThinking = selection?.config?.thinkingLevel
  const selectedThinking = reasoning?.options.some((option) => Object.is(option.value, currentThinking))
    ? encoded(currentThinking)
    : ""

  if (!models.length) {
    return <p className="text-xs text-muted-foreground">This host has not advertised any models.</p>
  }

  return (
    <div className="grid gap-2 sm:grid-cols-2">
      <label className="grid gap-1 text-xs font-medium" htmlFor={modelId}>
        Model
        <select
          id={modelId}
          value={selection?.id ?? ""}
          disabled={disabled}
          onChange={(event) => onChange(event.target.value ? modelSelection(event.target.value) : undefined)}
          className="h-9 rounded-md border bg-background px-2 text-sm disabled:cursor-not-allowed disabled:opacity-50"
        >
          {allowHostDefault
            ? <option value="">Host default</option>
            : <option value="" disabled>Choose a model</option>}
          {!selectedModel && selection && <option value={selection.id}>{selection.id} (unavailable)</option>}
          {models.map((model) => <option key={model.id} value={model.id}>{model.name || model.id}</option>)}
        </select>
      </label>
      {reasoning && selection && (
        <label className="grid gap-1 text-xs font-medium" htmlFor={reasoningId}>
          {reasoning.title}
          <select
            id={reasoningId}
            value={selectedThinking}
            disabled={disabled}
            title={reasoning.description}
            onChange={(event) => {
              const option = reasoning.options.find((candidate) => encoded(candidate.value) === event.target.value)
              onChange(modelSelectionWithThinking(selection, option?.value))
            }}
            className="h-9 rounded-md border bg-background px-2 text-sm disabled:cursor-not-allowed disabled:opacity-50"
          >
            <option value="">Model default{reasoning.default !== undefined ? ` (${String(reasoning.default)})` : ""}</option>
            {reasoning.options.map((option) => (
              <option key={encoded(option.value)} value={encoded(option.value)} title={option.description}>{option.label}</option>
            ))}
          </select>
        </label>
      )}
    </div>
  )
}
