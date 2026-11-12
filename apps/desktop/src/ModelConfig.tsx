import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";

export type ModelPreferences = {
  provider: "ollama" | "openai";
  localModel: string;
  cloudModel: string;
  visionModel: string;
};

function parseConfig(text: string): ModelPreferences {
  const value: unknown = JSON.parse(text);
  if (
    !value ||
    typeof value !== "object" ||
    !("version" in value) ||
    value.version !== 1 ||
    !("provider" in value) ||
    (value.provider !== "ollama" && value.provider !== "openai") ||
    !("localModel" in value) ||
    typeof value.localModel !== "string" ||
    !("cloudModel" in value) ||
    typeof value.cloudModel !== "string" ||
    !("visionModel" in value) ||
    typeof value.visionModel !== "string" ||
    Object.keys(value).length !== 5
  ) {
    throw new Error("Invalid model configuration");
  }
  return {
    provider: value.provider,
    localModel: value.localModel,
    cloudModel: value.cloudModel,
    visionModel: value.visionModel,
  };
}

export function ModelConfig({
  preferences,
  disabled,
  onImported,
}: {
  preferences: ModelPreferences;
  disabled: boolean;
  onImported: (preferences: ModelPreferences) => void;
}) {
  const [text, setText] = useState("");
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);

  async function importConfig() {
    setBusy(true);
    try {
      const imported = parseConfig(text);
      await invoke("set_model_preferences", { preferences: imported });
      onImported(imported);
      setStatus("Configuration importée.");
    } catch {
      setStatus("Configuration invalide ou impossible à enregistrer.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <details className="card">
      <summary>Importer ou exporter les choix de modèles</summary>
      <div className="task-form">
        <label htmlFor="model-config">Configuration des modèles (JSON)</label>
        <textarea
          id="model-config"
          value={text}
          maxLength={4096}
          rows={7}
          disabled={busy || disabled}
          onChange={(event) => {
            setText(event.target.value);
            setStatus("");
          }}
        />
        <div className="section-header">
          <button
            type="button"
            disabled={busy || disabled}
            onClick={() => {
              setText(JSON.stringify({ version: 1, ...preferences }, null, 2));
              setStatus("Configuration prête à copier.");
            }}
          >
            Exporter dans le champ
          </button>
          <button type="button" disabled={busy || disabled || !text.trim()} onClick={() => void importConfig()}>
            Appliquer cette configuration
          </button>
        </div>
        <p role="status">{status}</p>
      </div>
    </details>
  );
}
