import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type Probe = { toolCallVerified: boolean; toolCallAdvertised: boolean; visionAdvertised: boolean; elapsedMs: number };
type ToolCalling = "unverified" | "notAdvertised" | "verified" | "failed";
type Manifest = {
  name: string;
  toolCalling: ToolCalling;
  visionAdvertised: boolean;
  probeMs: number | null;
  probedAt: string | null;
};

export const toolCallingLabels: Record<ToolCalling, string> = {
  unverified: "Non vérifiés",
  notAdvertised: "Non annoncés (exclu du routage)",
  verified: "Vérifiés",
  failed: "Test échoué (exclu du routage)",
};

export function ModelProbe({ model, disabled }: { model: string; disabled: boolean }) {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Probe>();
  const [error, setError] = useState("");
  const [manifests, setManifests] = useState<Manifest[]>([]);

  async function loadManifests() {
    try {
      setManifests(await invoke<Manifest[]>("get_model_manifests"));
    } catch {
      setManifests([]);
    }
  }

  useEffect(() => {
    void loadManifests();
  }, []);

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
      void loadManifests();
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
      {manifests.length > 0 && (
        <table>
          <caption>Manifestes des modèles locaux testés</caption>
          <thead>
            <tr>
              <th scope="col">Modèle</th>
              <th scope="col">Appels d’outils</th>
              <th scope="col">Vision</th>
              <th scope="col">Dernier test</th>
            </tr>
          </thead>
          <tbody>
            {manifests.map((manifest) => (
              <tr key={manifest.name}>
                <th scope="row">{manifest.name}</th>
                <td>{toolCallingLabels[manifest.toolCalling]}</td>
                <td>{manifest.visionAdvertised ? "Annoncée" : "Non annoncée"}</td>
                <td>{manifest.probedAt ? new Date(manifest.probedAt).toLocaleString("fr-FR") : "—"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
