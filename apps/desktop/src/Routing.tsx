import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { placementAdvice, placementLabel, type LoadedModel } from "./placement";

const tasks: { name: string; status: string }[] = [
  { name: "Texte et actions Windows", status: "Modèle choisi ci-dessous (local ou cloud)" },
  { name: "Vision (description de fenêtre)", status: "Modèle de vision local choisi ci-dessous" },
  { name: "Synthèse vocale", status: "Voix Windows locale (français si installée)" },
  { name: "Reconnaissance vocale", status: "Moteur Windows local (SAPI), bouton Dicter de l’Accueil" },
  { name: "Génération d’images", status: "Moteur Stable Diffusion local (configuré ci-dessous)" },
  { name: "Génération vidéo", status: "ComfyUI local, dans la page Vidéo" },
];

const gb = (bytes: number) => (bytes / 1024 ** 3).toFixed(1);

export type LocalStrategy = "manual" | "adaptive" | "qualityFirst" | "costOptimized";

const strategies: { id: LocalStrategy; label: string; description: string }[] = [
  { id: "manual", label: "Manuel", description: "toujours le modèle Ollama choisi ci-dessous." },
  {
    id: "adaptive",
    label: "Adaptatif",
    description: "le modèle déjà le plus chargé sur le GPU, pour répondre plus vite.",
  },
  {
    id: "qualityFirst",
    label: "Qualité d’abord",
    description: "le plus gros modèle local installé capable d’appeler des outils.",
  },
  {
    id: "costOptimized",
    label: "Économie",
    description: "un modèle déjà chargé en mémoire, le plus léger, pour ne rien charger de nouveau.",
  },
];

/** Model per task, routing strategy and resources used by local models (Models page). */
export function Routing({
  onFallbackChange,
  strategy,
  onStrategyChange,
}: {
  onFallbackChange: (enabled: boolean) => void;
  strategy: LocalStrategy | undefined;
  onStrategyChange: (strategy: LocalStrategy) => void;
}) {
  const [fallback, setFallback] = useState<boolean>();
  const [routingStatus, setRoutingStatus] = useState("");
  const [loaded, setLoaded] = useState<LoadedModel[]>();
  const [status, setStatus] = useState("");
  const [budget, setBudget] = useState<number>();
  const [imageEndpoint, setImageEndpoint] = useState("");
  const [imageStatus, setImageStatus] = useState("");

  async function checkImageEngine() {
    try {
      const engine = await invoke<{ endpoint: string; available: boolean }>("get_image_engine");
      setImageEndpoint(engine.endpoint);
      setImageStatus(
        engine.endpoint ? (engine.available ? "Moteur d’images détecté." : "Moteur d’images injoignable.") : "",
      );
    } catch {
      setImageStatus("");
    }
  }

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
    void checkImageEngine();
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
      <fieldset className="task-form" disabled={strategy === undefined}>
        <legend>Choix du modèle local</legend>
        {strategies.map((option) => (
          <label key={option.id} className="approval">
            <input
              type="radio"
              name="local-strategy"
              value={option.id}
              checked={strategy === option.id}
              onChange={() => {
                setRoutingStatus("");
                void invoke("set_local_strategy", { strategy: option.id }).then(
                  () => onStrategyChange(option.id),
                  () => setRoutingStatus("Le réglage du routage n’a pas pu être enregistré."),
                );
              }}
            />
            <span>
              <strong>{option.label}</strong> — {option.description}
            </span>
          </label>
        ))}
        <p>
          Un autre modèle que celui choisi n’est utilisé qu’après un test d’appel d’outil réussi. Tout reste local ; la
          raison du choix s’affiche à chaque demande.
        </p>
      </fieldset>
      <p role="status">{routingStatus}</p>
      <label className="approval">
        <input
          type="checkbox"
          checked={fallback ?? false}
          disabled={fallback === undefined || (strategy !== undefined && strategy !== "manual")}
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
      <div className="task-form">
        <label htmlFor="image-endpoint">Moteur d’images local (API AUTOMATIC1111, sur ce PC uniquement)</label>
        <input
          id="image-endpoint"
          value={imageEndpoint}
          placeholder="http://127.0.0.1:7860 ; vide = aucun"
          onChange={(event) => setImageEndpoint(event.target.value)}
          onBlur={() => {
            void invoke("set_image_engine", { endpoint: imageEndpoint }).then(
              () => void checkImageEngine(),
              () =>
                setImageStatus("Adresse refusée : seul http://127.0.0.1:port ou http://localhost:port est accepté."),
            );
          }}
        />
        <div className="section-header">
          <button type="button" onClick={() => void invoke("open_images_folder")}>
            Ouvrir le dossier d’images
          </button>
        </div>
        <p role="status">{imageStatus}</p>
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
                {gb(model.size)} Go dont {gb(model.size_vram)} Go en mémoire GPU · {placementLabel(model)}
              </span>
            </li>
          ))}
        </ul>
      )}
      {loaded && placementAdvice(loaded) && <p role="note">{placementAdvice(loaded)}</p>}
      <p role="status">{status}</p>
    </section>
  );
}
