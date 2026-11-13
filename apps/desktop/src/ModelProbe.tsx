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
  contextLength: number | null;
  parameterSize: string | null;
  quantization: string | null;
  sizeBytes: number | null;
  tokensPerSecond: number | null;
  vramBytes: number | null;
};

const number = new Intl.NumberFormat("fr-FR", { maximumFractionDigits: 1 });

function hardware(manifest: Manifest): string {
  const parts = [manifest.parameterSize, manifest.quantization];
  if (manifest.sizeBytes) parts.push(`${number.format(manifest.sizeBytes / 1e9)} Go sur disque`);
  if (manifest.vramBytes) parts.push(`${number.format(manifest.vramBytes / 1e9)} Go en VRAM`);
  return parts.filter(Boolean).join(" · ") || "—";
}

export const toolCallingLabels: Record<ToolCalling, string> = {
  unverified: "Non vérifiés",
  notAdvertised: "Non annoncés (exclu du routage)",
  verified: "Vérifiés",
  failed: "Test échoué (exclu du routage)",
};

export function ModelProbe({ model, disabled }: { model: string; disabled: boolean }) {
  const [busy, setBusy] = useState(false);
  const [benchmarkProgress, setBenchmarkProgress] = useState("");
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
    setBenchmarkProgress("");
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

  async function runBenchmark() {
    const names = Array.from(new Set([model, ...manifests.map((item) => item.name)].filter(Boolean)));
    if (names.length === 0) return;
    setBusy(true);
    setError("");
    setResult(undefined);
    try {
      for (let index = 0; index < names.length; index += 1) {
        setBenchmarkProgress(`Banc de performance : ${names[index]} (${index + 1}/${names.length})…`);
        setResult(await invoke<Probe>("probe_local_model", { model: names[index] }));
      }
      setBenchmarkProgress(`Banc de performance terminé (${names.length} modèles mesurés).`);
    } catch {
      setError("Le banc de performance a été interrompu. Vérifie qu’Ollama est accessible.");
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
      <button
        type="button"
        disabled={disabled || busy || (!model && manifests.length === 0)}
        onClick={() => void runBenchmark()}
      >
        Banc de performance
      </button>
      <div role="status">
        {busy && <p>{benchmarkProgress || "Test en cours…"}</p>}
        {!busy && benchmarkProgress && <p>{benchmarkProgress}</p>}
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
              <th scope="col">Contexte</th>
              <th scope="col">Taille</th>
              <th scope="col">Débit</th>
              <th scope="col">Dernier test</th>
            </tr>
          </thead>
          <tbody>
            {manifests.map((manifest) => (
              <tr key={manifest.name}>
                <th scope="row">{manifest.name}</th>
                <td>{toolCallingLabels[manifest.toolCalling]}</td>
                <td>{manifest.visionAdvertised ? "Annoncée" : "Non annoncée"}</td>
                <td>{manifest.contextLength ? `${number.format(manifest.contextLength)} jetons` : "—"}</td>
                <td>{hardware(manifest)}</td>
                <td>{manifest.tokensPerSecond ? `${manifest.tokensPerSecond} jetons/s` : "—"}</td>
                <td>{manifest.probedAt ? new Date(manifest.probedAt).toLocaleString("fr-FR") : "—"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
