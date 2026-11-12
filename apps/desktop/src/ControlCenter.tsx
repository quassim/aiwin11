import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { type LoadedModel, placementAdvice, placementLabel } from "./placement";
import { Updates } from "./Updates";

type Policy = { settings: { autonomy: string; simulate: boolean } };
type CloudUsage = { tokens: number; tasks: number };
type RuntimeStatus = {
  version: string;
  windowsBridge: boolean;
  agentScript: boolean;
  python: boolean;
  adminHelper: boolean;
};
const rateKey = "aiwin11-cloud-eur-per-million";

const profiles: Record<string, string> = {
  observation: "Observation",
  assisted: "Assisté",
  limited: "Autonome limité",
  advanced: "Avancé",
};

export function ControlCenter({
  model,
  provider,
  agent,
  status,
  running,
  latestTask,
  onModels,
  onSecurity,
  onActivity,
}: {
  model: string;
  provider: string;
  agent: string;
  status: string;
  running: boolean;
  latestTask?: string;
  onModels: () => void;
  onSecurity: () => void;
  onActivity: () => void;
}) {
  const [policy, setPolicy] = useState<Policy>();
  const [privacy, setPrivacy] = useState<boolean>();
  const [loaded, setLoaded] = useState<LoadedModel[]>();
  const [usage, setUsage] = useState<CloudUsage>();
  const [runtime, setRuntime] = useState<RuntimeStatus>();
  const [rate, setRate] = useState(() => localStorage.getItem(rateKey) ?? "");
  const [loading, setLoading] = useState(true);
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    let current = true;
    void Promise.allSettled([
      invoke<Policy>("get_policy_overview"),
      invoke<boolean>("get_privacy"),
      invoke<LoadedModel[]>("get_model_resources"),
      invoke<CloudUsage>("get_cloud_usage"),
      invoke<RuntimeStatus>("get_runtime_status"),
    ]).then(([nextPolicy, nextPrivacy, nextLoaded, nextUsage, nextRuntime]) => {
      if (!current) return;
      setPolicy(nextPolicy.status === "fulfilled" ? nextPolicy.value : undefined);
      setPrivacy(nextPrivacy.status === "fulfilled" ? nextPrivacy.value : undefined);
      setLoaded(nextLoaded.status === "fulfilled" ? nextLoaded.value : undefined);
      setUsage(nextUsage.status === "fulfilled" ? nextUsage.value : undefined);
      setRuntime(nextRuntime.status === "fulfilled" ? nextRuntime.value : undefined);
      setLoading(false);
    });
    return () => {
      current = false;
    };
  }, [revision]);

  return (
    <section className="card" aria-labelledby="control-status">
      <div className="section-header">
        <h2 id="control-status">État d’AiWin11</h2>
        <button
          type="button"
          disabled={loading}
          onClick={() => {
            setLoading(true);
            setRevision((value) => value + 1);
          }}
        >
          Actualiser
        </button>
      </div>
      <dl>
        <dt>Agent</dt>
        <dd>
          {agent} · {status}
        </dd>
        <dt>Dernière demande</dt>
        <dd>{latestTask ?? "Aucune demande enregistrée"}</dd>
        <dt>Fournisseur sélectionné</dt>
        <dd>
          {provider}
          {model && ` · ${model}`}
        </dd>
        <dt>Mode privé</dt>
        <dd>{privacy === undefined ? "Indisponible" : privacy ? "Actif" : "Désactivé"}</dd>
        <dt>Profil d’autonomie</dt>
        <dd>{policy ? (profiles[policy.settings.autonomy] ?? policy.settings.autonomy) : "Indisponible"}</dd>
        <dt>Simulation</dt>
        <dd>{policy ? (policy.settings.simulate ? "Active" : "Désactivée") : "Indisponible"}</dd>
      </dl>
      <h2>Modèles chargés dans Ollama</h2>
      {loading ? (
        <p role="status">Actualisation…</p>
      ) : loaded?.length ? (
        <>
          <ul>
            {loaded.map((model) => (
              <li key={model.name}>
                {model.name}{" "}
                <span>
                  {(model.size_vram / 1024 ** 3).toLocaleString("fr-FR", { maximumFractionDigits: 1 })} Gio de VRAM ·{" "}
                  {placementLabel(model)}
                </span>
              </li>
            ))}
          </ul>
          {placementAdvice(loaded) && <p role="note">{placementAdvice(loaded)}</p>}
        </>
      ) : (
        <p>{loaded ? "Aucun modèle chargé." : "Ollama est indisponible."}</p>
      )}
      <h2>Consommation cloud</h2>
      {usage ? (
        <>
          <p>
            {usage.tokens.toLocaleString("fr-FR")} jetons · {usage.tasks.toLocaleString("fr-FR")} demandes enregistrées
            sur 30 jours.
          </p>
          <label htmlFor="cloud-rate">Tarif moyen estimé en euros par million de jetons</label>
          <input
            id="cloud-rate"
            type="number"
            min="0"
            max="100000"
            step="any"
            value={rate}
            onChange={(event) => {
              const value = event.target.value;
              setRate(value);
              if (event.target.validity.valid) localStorage.setItem(rateKey, value);
            }}
          />
          {rate !== "" && Number(rate) >= 0 && Number(rate) <= 100000 && (
            <p>
              Coût estimé{" "}
              {((usage.tokens * Number(rate)) / 1_000_000).toLocaleString("fr-FR", {
                style: "currency",
                currency: "EUR",
                maximumFractionDigits: 4,
              })}
            </p>
          )}
        </>
      ) : (
        <p>{loading ? "Chargement de la consommation…" : "Consommation indisponible."}</p>
      )}
      <div className="section-header">
        <button type="button" onClick={onModels}>
          Modèles
        </button>
        <button type="button" onClick={onSecurity}>
          Sécurité
        </button>
        <button type="button" onClick={onActivity}>
          Activité
        </button>
      </div>
      <h2>Installation</h2>
      {runtime ? (
        <>
          <p>AiWin11 {runtime.version}</p>
          <ul>
            {(
              [
                ["Pont Windows", runtime.windowsBridge],
                ["Script de l’agent", runtime.agentScript],
                ["Interpréteur Python", runtime.python],
                ["Assistant administrateur", runtime.adminHelper],
              ] as const
            ).map(([label, detected]) => (
              <li key={label}>
                {label} <strong>{detected ? "Détecté" : "Introuvable"}</strong>
              </li>
            ))}
          </ul>
        </>
      ) : (
        <p>{loading ? "Recherche des composants…" : "État de l’installation indisponible."}</p>
      )}
      <Updates running={running} />
    </section>
  );
}
