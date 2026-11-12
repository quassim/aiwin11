import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { CloudCostSettings } from "./CloudCost";
import { LocalFallback } from "./LocalFallback";
import { LocalVoiceSetting } from "./Speech";
import { placementAdvice, placementLabel, type LoadedModel } from "./placement";

const tasks: { name: string; status: string }[] = [
  { name: "Texte et actions Windows", status: "Modèle choisi ci-dessous (local ou cloud)" },
  { name: "Vision (description de fenêtre)", status: "Modèle de vision local choisi ci-dessous" },
  { name: "Synthèse vocale", status: "Voix Windows locale (français si installée)" },
  { name: "Reconnaissance vocale", status: "Moteur Windows local (SAPI), bouton Dicter de l’Accueil" },
  { name: "Génération d’images", status: "Moteur Stable Diffusion local (configuré ci-dessous)" },
  { name: "Génération vidéo", status: "ComfyUI local, dans la page Vidéo" },
  { name: "Embeddings de la mémoire", status: "Modèle Ollama sélectionné dans la page Mémoire" },
];

const gb = (bytes: number) => (bytes / 1024 ** 3).toFixed(1);

export type LocalStrategy =
  "manual" | "localOnly" | "cloudOnly" | "localFirst" | "balanced" | "adaptive" | "qualityFirst" | "costOptimized";
export type AgentModels = {
  office: string;
  windowsAdmin: string;
  development: string;
  research: string;
  multimedia: string;
  readOnly: string;
};
type ImageBackend = "automatic1111" | "comfyUi";
export type CloudProvider = { id: string; label: string; enabled: boolean };
export type LocalRuntimePreferences = { runtime: "ollama" | "lmStudio"; lmStudioModel: string };
const agentProfiles = [
  { key: "office", label: "Fichiers et bureautique" },
  { key: "windowsAdmin", label: "Administration Windows" },
  { key: "development", label: "Développement" },
  { key: "research", label: "Recherche" },
  { key: "multimedia", label: "Multimédia" },
  { key: "readOnly", label: "Lecture seule" },
] as const;

const strategies: { id: LocalStrategy; label: string; description: string }[] = [
  { id: "manual", label: "Manuel", description: "utilise le fournisseur et le modèle choisis." },
  { id: "localOnly", label: "Local uniquement", description: "utilise Ollama et interdit le repli cloud." },
  {
    id: "cloudOnly",
    label: "Cloud uniquement",
    description: "utilise le fournisseur cloud choisi, avec accord par demande.",
  },
  {
    id: "localFirst",
    label: "Local d’abord",
    description: "repli cloud sur accord si Ollama est injoignable avant toute action.",
  },
  {
    id: "balanced",
    label: "Mixte",
    description:
      "privilégie un modèle vérifié déjà sur le GPU. L’agent général peut utiliser le modèle cloud vérifié sur accord.",
  },
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
  models,
  currentModel,
  agentModels,
  onAgentModelsChange,
  cloudProviders,
  onCloudProviderChange,
  localRuntime,
  onLocalRuntimeChange,
}: {
  models: string[];
  currentModel: string;
  onFallbackChange: (enabled: boolean) => void;
  strategy: LocalStrategy | undefined;
  onStrategyChange: (strategy: LocalStrategy) => void;
  agentModels: AgentModels | undefined;
  onAgentModelsChange: (models: AgentModels) => void;
  cloudProviders: CloudProvider[];
  onCloudProviderChange: (id: string, enabled: boolean) => void;
  localRuntime: LocalRuntimePreferences;
  onLocalRuntimeChange: (preferences: LocalRuntimePreferences) => void;
}) {
  const [fallback, setFallback] = useState<boolean>();
  const [routingStatus, setRoutingStatus] = useState("");
  const [loaded, setLoaded] = useState<LoadedModel[]>();
  const [status, setStatus] = useState("");
  const [budget, setBudget] = useState<number>();
  const [imageEndpoint, setImageEndpoint] = useState("");
  const [imageBackend, setImageBackend] = useState<ImageBackend>("automatic1111");
  const [savingImage, setSavingImage] = useState(false);
  const [imageStatus, setImageStatus] = useState("");
  const [checkpoint, setCheckpoint] = useState("");
  const [checkpoints, setCheckpoints] = useState<{ id: string; label: string }[]>([]);
  const [savingProfiles, setSavingProfiles] = useState(false);
  const [speech, setSpeech] = useState<{
    selected: string;
    recognizers: { id: string; label: string; language: string }[];
  }>();
  const [speechStatus, setSpeechStatus] = useState("");
  const [lmModels, setLmModels] = useState<string[]>([]);
  const [lmStatus, setLmStatus] = useState("");

  function saveLocalRuntime(preferences: LocalRuntimePreferences) {
    void invoke("set_local_runtime_preferences", { preferences }).then(
      () => {
        onLocalRuntimeChange(preferences);
        setLmStatus("");
      },
      () => setLmStatus("Le moteur local n’a pas pu être enregistré."),
    );
  }

  async function checkImageEngine() {
    try {
      const engine = await invoke<{
        endpoint: string;
        backend: ImageBackend;
        available: boolean;
        checkpoint: string | null;
        checkpoints: { id: string; label: string }[];
      }>("get_image_engine");
      setImageEndpoint(engine.endpoint);
      setImageBackend(engine.backend);
      setCheckpoint(engine.checkpoint ?? "");
      setCheckpoints(engine.checkpoints);
      setImageStatus(
        engine.endpoint ? (engine.available ? "Moteur d’images détecté." : "Moteur d’images injoignable.") : "",
      );
    } catch {
      setImageStatus("");
    }
  }

  async function saveImageEngine() {
    setSavingImage(true);
    try {
      await invoke("set_image_engine", { endpoint: imageEndpoint, backend: imageBackend });
      await checkImageEngine();
    } catch {
      setImageStatus("Adresse refusée : seul http://127.0.0.1:port ou http://localhost:port est accepté.");
    } finally {
      setSavingImage(false);
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
    void invoke<typeof speech>("get_speech_recognizers").then(setSpeech, () =>
      setSpeechStatus("Moteurs de dictée indisponibles."),
    );
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
      <LocalVoiceSetting />
      <label>
        Moteur de dictée Windows
        <select
          value={speech?.selected ?? ""}
          disabled={!speech}
          onChange={(event) => {
            if (!speech) return;
            const id = event.target.value;
            void invoke("set_speech_recognizer", { id }).then(
              () => {
                setSpeech({ ...speech, selected: id });
                setSpeechStatus("");
              },
              () => setSpeechStatus("Ce moteur de dictée n’a pas pu être sélectionné."),
            );
          }}
        >
          <option value="">Automatique</option>
          {speech?.selected && !speech.recognizers.some((entry) => entry.id === speech.selected) && (
            <option value={speech.selected}>Moteur enregistré indisponible</option>
          )}
          {speech?.recognizers.map((entry) => (
            <option key={entry.id} value={entry.id}>
              {entry.label} ({entry.language})
            </option>
          ))}
        </select>
      </label>
      <p role="status">{speechStatus}</p>
      <fieldset className="task-form">
        <legend>Moteur local de texte et d’actions</legend>
        <label>
          Moteur local
          <select
            value={localRuntime.runtime}
            onChange={(event) =>
              saveLocalRuntime({
                ...localRuntime,
                runtime: event.target.value === "lmStudio" ? "lmStudio" : "ollama",
              })
            }
          >
            <option value="ollama">Ollama</option>
            <option value="lmStudio">LM Studio</option>
          </select>
        </label>
        {localRuntime.runtime === "lmStudio" && (
          <>
            <button
              type="button"
              onClick={() => {
                setLmStatus("");
                void invoke<string[]>("discover_lm_studio_models").then(
                  (list) => {
                    setLmModels(list);
                    if (!list.length) setLmStatus("Aucun modèle LM Studio disponible.");
                  },
                  () => setLmStatus("Serveur LM Studio indisponible sur 127.0.0.1:1234."),
                );
              }}
            >
              Lister les modèles LM Studio
            </button>
            <label>
              Modèle LM Studio
              <select
                value={localRuntime.lmStudioModel}
                onChange={(event) => saveLocalRuntime({ ...localRuntime, lmStudioModel: event.target.value })}
              >
                <option value="">Sélectionner un modèle</option>
                {[...new Set([...lmModels, localRuntime.lmStudioModel].filter(Boolean))].map((id) => (
                  <option key={id} value={id}>
                    {id}
                  </option>
                ))}
              </select>
            </label>
          </>
        )}
        <p role="status">{lmStatus}</p>
      </fieldset>
      <fieldset>
        <legend>Fournisseurs cloud autorisés</legend>
        {cloudProviders.map((entry) => (
          <label className="approval" key={entry.id}>
            <input
              type="checkbox"
              checked={entry.enabled}
              onChange={(event) => {
                const enabled = event.target.checked;
                void invoke("set_cloud_provider_enabled", { vendor: entry.id, enabled }).then(
                  () => onCloudProviderChange(entry.id, enabled),
                  () => setRoutingStatus("Le fournisseur n’a pas pu être modifié."),
                );
              }}
            />
            {entry.label}
          </label>
        ))}
      </fieldset>
      <fieldset className="task-form" disabled={strategy === undefined}>
        <legend>Stratégie de routage</legend>
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
          Le classement Ollama teste les appels d’outils avant de choisir un autre modèle. Le cloud demande ton accord
          pour chaque tâche.
        </p>
      </fieldset>
      <fieldset className="task-form" disabled={!agentModels || savingProfiles}>
        <legend>Modèle Ollama par agent</legend>
        {agentProfiles.map(({ key, label }) => {
          const selected = agentModels?.[key] ?? "";
          const available = [...new Set([...models, selected].filter(Boolean))];
          return (
            <label key={key}>
              {label}
              <select
                value={selected}
                onChange={(event) => {
                  if (!agentModels) return;
                  const next = { ...agentModels, [key]: event.target.value };
                  setSavingProfiles(true);
                  setRoutingStatus("");
                  void invoke("set_agent_models", { models: next })
                    .then(
                      () => onAgentModelsChange(next),
                      () => setRoutingStatus("Le modèle de cet agent n’a pas pu être enregistré."),
                    )
                    .finally(() => setSavingProfiles(false));
                }}
              >
                <option value="">Modèle général</option>
                {available.map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
            </label>
          );
        })}
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
      <LocalFallback models={models} current={currentModel} />
      <CloudCostSettings />
      <div className="task-form">
        <label htmlFor="image-backend">Moteur d’images local</label>
        <select
          id="image-backend"
          value={imageBackend}
          disabled={savingImage}
          onChange={(event) => {
            setImageBackend(event.target.value === "comfyUi" ? "comfyUi" : "automatic1111");
            setCheckpoints([]);
            setCheckpoint("");
          }}
        >
          <option value="automatic1111">AUTOMATIC1111</option>
          <option value="comfyUi">ComfyUI (checkpoint Stable Diffusion)</option>
        </select>
        <label htmlFor="image-endpoint">Adresse locale du moteur</label>
        <input
          id="image-endpoint"
          value={imageEndpoint}
          placeholder={imageBackend === "comfyUi" ? "http://127.0.0.1:8188" : "http://127.0.0.1:7860"}
          disabled={savingImage}
          onChange={(event) => setImageEndpoint(event.target.value)}
        />
        <button type="button" disabled={savingImage} onClick={() => void saveImageEngine()}>
          Enregistrer le moteur
        </button>
        <label htmlFor="image-checkpoint">Modèle d’images</label>
        <select
          id="image-checkpoint"
          value={checkpoint}
          disabled={savingImage || !checkpoints.length}
          onChange={(event) => {
            const next = event.target.value;
            void invoke("set_image_checkpoint", { checkpoint: next }).then(
              () => setCheckpoint(next),
              () => setImageStatus("Le modèle d’images n’a pas pu être enregistré."),
            );
          }}
        >
          <option value="">
            {imageBackend === "comfyUi" ? "Sélectionner un checkpoint" : "Modèle par défaut du moteur"}
          </option>
          {checkpoint && !checkpoints.some((entry) => entry.id === checkpoint) && (
            <option value={checkpoint}>Modèle enregistré indisponible</option>
          )}
          {checkpoints.map((entry) => (
            <option key={entry.id} value={entry.id}>
              {entry.label}
            </option>
          ))}
        </select>
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
