import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type LoadedModel = { name: string; size: number; size_vram: number };

const tasks: { name: string; status: string }[] = [
  { name: "Texte et actions Windows", status: "Modèle choisi ci-dessous (local ou cloud)" },
  { name: "Vision (description de fenêtre)", status: "Modèle de vision local choisi ci-dessous" },
  { name: "Synthèse vocale", status: "Voix Windows locale (français si installée)" },
  { name: "Reconnaissance vocale", status: "Pas encore disponible : aucun moteur local (phase R7)" },
  { name: "Génération d’images", status: "Pas encore disponible (phase R7)" },
  { name: "Génération vidéo", status: "Pas encore disponible (phase R7)" },
];

const gb = (bytes: number) => (bytes / 1024 ** 3).toFixed(1);

/** Model per task, routing strategy and resources used by local models (Models page). */
export function Routing({ onFallbackChange }: { onFallbackChange: (enabled: boolean) => void }) {
  const [fallback, setFallback] = useState<boolean>();
  const [loaded, setLoaded] = useState<LoadedModel[]>();
  const [status, setStatus] = useState("");
  const [budget, setBudget] = useState<number>();

  async function refreshResources() {
    try {
      setLoaded(await invoke<LoadedModel[]>("get_model_resources"));
      setStatus("");
    } catch {
      setLoaded(undefined);
      setStatus("Ollama est indisponible : ressources inconnues.");
    }
  }

  useEffect(() => {
    void invoke<boolean>("get_cloud_fallback").then((enabled) => {
      setFallback(enabled);
      onFallbackChange(enabled);
    });
    void invoke<number>("get_cloud_budget").then(setBudget);
    void refreshResources();
  }, [onFallbackChange]);

  return (
    <section className="card" aria-labelledby="routing-heading">
      <h2 id="routing-heading">Modèles par tâche</h2>
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              <th scope="col">Tâche</th>
              <th scope="col">Modèle</th>
            </tr>
          </thead>
          <tbody>
            {tasks.map((task) => (
              <tr key={task.name}>
                <th scope="row">{task.name}</th>
                <td>{task.status}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <label className="approval">
        <input
          type="checkbox"
          checked={fallback ?? false}
          disabled={fallback === undefined}
          onChange={(event) => {
            const next = event.target.checked;
            void invoke("set_cloud_fallback", { enabled: next }).then(() => {
              setFallback(next);
              onFallbackChange(next);
            });
          }}
        />
        <span>
          <strong>Local d’abord, cloud en secours</strong> — si Ollama ne répond pas, la demande peut passer à le modèle
          cloud saisi plus bas, uniquement si tu coches l’envoi pour cette demande, avant toute action, et jamais en
          mode privé. Sinon, le modèle choisi est utilisé tel quel.
        </span>
      </label>
      <div className="task-form">
        <label htmlFor="cloud-budget">Budget maximal d’une demande envoyée au cloud (jetons)</label>
        <select
          id="cloud-budget"
          value={budget ?? ""}
          disabled={budget === undefined}
          onChange={(event) => {
            const tokens = Number(event.target.value);
            void invoke("set_cloud_budget", { tokens }).then(() => setBudget(tokens));
          }}
        >
          {[10_000, 20_000, 50_000, 100_000, 200_000].map((value) => (
            <option key={value} value={value}>
              {value.toLocaleString("fr-FR")} jetons
            </option>
          ))}
        </select>
      </div>
      <div className="section-header">
        <h3>Ressources des modèles locaux</h3>
        <button onClick={() => void refreshResources()}>Actualiser</button>
      </div>
      {loaded && loaded.length === 0 && <p>Aucun modèle chargé en mémoire.</p>}
      {loaded && loaded.length > 0 && (
        <ul>
          {loaded.map((model) => (
            <li key={model.name}>
              <span>{model.name}</span>
              <span>
                {gb(model.size)} Go dont {gb(model.size_vram)} Go en mémoire GPU
              </span>
            </li>
          ))}
        </ul>
      )}
      <p role="status">{status}</p>
    </section>
  );
}
