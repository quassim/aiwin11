import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type Grant = { id: string; action: string; createdAt: string; expiresAt: string };
type GrantOverview = { active: Grant[]; grantable: string[] };

const durations = [15, 30, 60, 120];

/** Temporary, revocable permissions (Security page). */
export function Grants({ actionNames, onChange }: { actionNames: Record<string, string>; onChange: () => void }) {
  const [overview, setOverview] = useState<GrantOverview>();
  const [action, setAction] = useState("");
  const [minutes, setMinutes] = useState(30);
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);

  async function load() {
    try {
      const next = await invoke<GrantOverview>("get_grants");
      setOverview(next);
      setAction((current) => current || next.grantable[0] || "");
    } catch {
      setStatus("Les autorisations temporaires sont indisponibles.");
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function act(run: () => Promise<string>) {
    setBusy(true);
    try {
      setStatus(await run());
    } catch {
      setStatus("L’opération a échoué.");
    } finally {
      setBusy(false);
      await load();
      onChange();
    }
  }

  const name = (code: string) => actionNames[code] ?? code;

  return (
    <section aria-labelledby="grants-heading">
      <h3 id="grants-heading">Autorisations temporaires</h3>
      <p>
        Laisse une action se faire sans demander pendant une durée limitée. Elle ne s’applique que là où ton profil
        demande ton accord : jamais aux actions critiques, à la lecture de l’écran, en mode simulation ni avec un modèle
        cloud pour les outils locaux. Tu peux la révoquer à tout moment ; la révocation vaut dès l’action suivante.
      </p>
      {overview && (
        <form
          className="task-form"
          onSubmit={(event) => {
            event.preventDefault();
            if (!action) return;
            void act(async () => {
              await invoke("create_grant", { action, minutes });
              return `${name(action)} : autorisé sans demander pendant ${minutes} minutes.`;
            });
          }}
        >
          <label htmlFor="grant-action">Action</label>
          <select id="grant-action" value={action} disabled={busy} onChange={(event) => setAction(event.target.value)}>
            {overview.grantable.map((code) => (
              <option key={code} value={code}>
                {name(code)}
              </option>
            ))}
          </select>
          <label htmlFor="grant-minutes">Durée</label>
          <select
            id="grant-minutes"
            value={minutes}
            disabled={busy}
            onChange={(event) => setMinutes(Number(event.target.value))}
          >
            {durations.map((value) => (
              <option key={value} value={value}>
                {value} minutes
              </option>
            ))}
          </select>
          <button type="submit" disabled={busy || !action}>
            Accorder
          </button>
        </form>
      )}
      {overview?.active.length ? (
        <ul>
          {overview.active.map((grant) => (
            <li key={grant.id}>
              <span>
                {name(grant.action)} — jusqu’à{" "}
                <time dateTime={grant.expiresAt}>{new Date(grant.expiresAt).toLocaleTimeString("fr-FR")}</time>
              </span>
              <button
                disabled={busy}
                onClick={() =>
                  void act(async () => {
                    await invoke("revoke_grant", { id: grant.id });
                    return `Autorisation révoquée : ${name(grant.action)}.`;
                  })
                }
              >
                Révoquer
              </button>
            </li>
          ))}
        </ul>
      ) : (
        overview && <p>Aucune autorisation temporaire en cours.</p>
      )}
      <p role="status">{status}</p>
    </section>
  );
}
