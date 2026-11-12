import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

const retentionChoices = [7, 30, 90, 365];

/** Retention, erasure and export of the audit history (Activity page). */
export function History({ running, onChange }: { running: boolean; onChange: () => void }) {
  const [days, setDays] = useState<number>();
  const [status, setStatus] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void invoke<{ retentionDays: number }>("get_history_settings").then(
      (settings) => setDays(settings.retentionDays),
      () => setStatus("La durée de conservation est indisponible."),
    );
  }, []);

  async function act(action: () => Promise<string>) {
    setBusy(true);
    try {
      setStatus(await action());
    } catch (failure) {
      setStatus(
        String(failure) === "TASK_BUSY"
          ? "Une demande est en cours : attends sa fin pour effacer l’historique."
          : "L’opération a échoué.",
      );
    } finally {
      setBusy(false);
      setConfirming(false);
      onChange();
    }
  }

  const choices =
    days === undefined || retentionChoices.includes(days) ? retentionChoices : [...retentionChoices, days];

  return (
    <section className="card" aria-labelledby="history-heading">
      <h2 id="history-heading">Historique et journal d’audit</h2>
      <p>
        Le journal ne contient que des codes (types d’événement, décisions, règles, erreurs), jamais tes demandes, les
        réponses ni le contenu des fichiers.
      </p>
      <div className="task-form">
        <label htmlFor="retention">Conserver l’historique des sessions passées pendant</label>
        <select
          id="retention"
          value={days ?? ""}
          disabled={days === undefined || busy}
          onChange={(event) => {
            const chosen = Number(event.target.value);
            void act(async () => {
              await invoke("set_retention_days", { days: chosen });
              setDays(chosen);
              return `Conservation réglée sur ${chosen} jours. Les sessions plus anciennes ont été supprimées.`;
            });
          }}
        >
          {choices.map((value) => (
            <option key={value} value={value}>
              {value} jours
            </option>
          ))}
        </select>
      </div>
      <div className="section-header">
        <button
          disabled={busy}
          onClick={() =>
            void act(async () => {
              const path = await invoke<string>("export_audit");
              return `Journal exporté : ${path}`;
            })
          }
        >
          Exporter le journal (JSON)
        </button>
        {!confirming ? (
          <button disabled={busy || running} onClick={() => setConfirming(true)}>
            Effacer l’historique…
          </button>
        ) : (
          <span role="group" aria-label="Confirmer l’effacement">
            <button autoFocus onClick={() => setConfirming(false)}>
              Garder
            </button>{" "}
            <button
              className="danger"
              disabled={busy}
              onClick={() =>
                void act(async () => {
                  const removed = await invoke<number>("clear_history");
                  return `Historique effacé (${removed} session${removed > 1 ? "s" : ""}). Le journal de la session en cours est conservé.`;
                })
              }
            >
              Effacer définitivement
            </button>
          </span>
        )}
      </div>
      {confirming && (
        <p role="alert">
          L’effacement supprime définitivement les demandes et le journal des sessions passées. Il ne peut pas être
          annulé.
        </p>
      )}
      <p role="status">{status}</p>
    </section>
  );
}
