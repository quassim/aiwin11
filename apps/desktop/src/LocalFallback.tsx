import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

/** Local model used once if the chosen local model fails on the first step. Never a cloud model. */
export function LocalFallback({ models, current }: { models: string[]; current: string }) {
  const [fallback, setFallback] = useState<string>();
  const [error, setError] = useState("");

  useEffect(() => {
    void invoke<string | null>("get_local_fallback_model")
      .then((name) => setFallback(name ?? ""))
      .catch(() => setError("Réglage indisponible."));
  }, []);

  function choose(name: string) {
    setError("");
    void invoke("set_local_fallback_model", { model: name })
      .then(() => setFallback(name))
      .catch(() => setError("Enregistrement impossible."));
  }

  // A fallback no longer installed stays listed so that it can be seen and removed.
  const options = fallback && !models.includes(fallback) ? [...models, fallback] : models;

  return (
    <div className="task-form">
      <label htmlFor="local-fallback">Modèle local de secours</label>
      <select
        id="local-fallback"
        value={fallback ?? ""}
        disabled={fallback === undefined}
        onChange={(event) => choose(event.target.value)}
      >
        <option value="">Aucun</option>
        {options.map((name) => (
          <option key={name} value={name} disabled={name === current}>
            {name}
            {models.includes(name) ? "" : " (non installé)"}
          </option>
        ))}
      </select>
      <p>
        Si le modèle choisi échoue avant la première étape (absent, supprimé ou défaillant), la demande continue une
        fois avec ce modèle, après un appel d’outil de test réussi. Il reste sur ce PC ; aucune bascule vers le cloud.
      </p>
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
