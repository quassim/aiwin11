import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { type CloudCost, euros, loadCloudCost } from "./CloudCost";
import { type LoadedModel, placementAdvice, placementLabel } from "./placement";
import { Backups } from "./Backups";
import { Updates } from "./Updates";

type Policy = { settings: { autonomy: string; simulate: boolean } };
type CloudUsage = { tokens: number; tasks: number };
type RuntimeStatus = {
  version: string;
  windowsBridge: boolean;
  agentScript: boolean;
  python: boolean;
  adminHelper: boolean;
  uiReadyMs: number | null;
};

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
  const [cost, setCost] = useState<CloudCost>();
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
      loadCloudCost(),
    ]).then(([nextPolicy, nextPrivacy, nextLoaded, nextUsage, nextRuntime, nextCost]) => {
      if (!current) return;
      setPolicy(nextPolicy.status === "fulfilled" ? nextPolicy.value : undefined);
      setPrivacy(nextPrivacy.status === "fulfilled" ? nextPrivacy.value : undefined);
      setLoaded(nextLoaded.status === "fulfilled" ? nextLoaded.value : undefined);
      setUsage(nextUsage.status === "fulfilled" ? nextUsage.value : undefined);
      setRuntime(nextRuntime.status === "fulfilled" ? nextRuntime.value : undefined);
      setCost(nextCost.status === "fulfilled" ? nextCost.value : undefined);
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
          {cost?.priceMilliPerMillion ? (
            <p>
              Coût estimé {euros(Math.ceil((usage.tokens * cost.priceMilliPerMillion) / 1_000_000))} au tarif saisi
              {cost.maxMilliPerTask ? `, plafond de ${euros(cost.maxMilliPerTask)} par demande` : ""}.
            </p>
          ) : (
            <p>Saisis le tarif de ton fournisseur dans la page Modèles pour estimer le coût.</p>
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
          <p>
            AiWin11 {runtime.version}
            {runtime.uiReadyMs !== null &&
              ` · interface prête en ${(runtime.uiReadyMs / 1000).toLocaleString("fr-FR", { maximumFractionDigits: 2 })} s`}
          </p>
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
      <Backups disabled={running} />
    </section>
  );
}
