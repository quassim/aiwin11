import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";

type Probe = { toolCallVerified: boolean; toolCallAdvertised: boolean; visionAdvertised: boolean; elapsedMs: number };

export function ModelProbe({ model, disabled }: { model: string; disabled: boolean }) {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Probe>();
  const [error, setError] = useState("");

  async function testModel() {
    setBusy(true);
    setError("");
    setResult(undefined);
    try {
      setResult(await invoke<Probe>("probe_local_model", { model }));
    } catch {
      setError("Le test a échoué. Vérifie que le modèle est installé et qu’Ollama est accessible.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div>
      <button type="button" disabled={disabled || busy || !model} onClick={() => void testModel()}>
        Tester le modèle local
      </button>
      <div role="status">
        {busy && <p>Test en cours…</p>}
        {error && <p>{error}</p>}
        {result && (
          <ul>
            <li>
              Appels d’outils{" "}
              <strong>
                {result.toolCallVerified
                  ? "Vérifiés"
                  : result.toolCallAdvertised
                    ? "Test non concluant"
                    : "Non annoncés"}
              </strong>
            </li>
            <li>
              Vision <strong>{result.visionAdvertised ? "Annoncée par Ollama" : "Non annoncée"}</strong>
            </li>
            <li>
              Durée du test{" "}
              <strong>{(result.elapsedMs / 1000).toLocaleString("fr-FR", { maximumFractionDigits: 2 })} s</strong>
            </li>
          </ul>
        )}
      </div>
    </div>
  );
}
