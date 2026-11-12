import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type Routine = { id: string; text: string; createdAt: string; schedule?: string };

/**
 * Saved requests (encrypted). One click puts one back in the request field. A daily time makes
 * AiWin11 run it alone with the local model in the Observation profile (reads only).
 */
export function Routines({ prompt, onPick }: { prompt: string; onPick: (text: string) => void }) {
  const [routines, setRoutines] = useState<Routine[]>([]);
  const [reports, setReports] = useState<Routine[]>([]);
  const [times, setTimes] = useState<Record<string, string>>({});
  const [status, setStatus] = useState("");

  async function load() {
    try {
      setRoutines(await invoke<Routine[]>("get_routines"));
      setReports((await invoke<Routine[]>("get_reports")).reverse());
    } catch {
      setRoutines([]);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function save() {
    try {
      await invoke("save_routine", { text: prompt });
      setStatus("Demande enregistrée.");
      await load();
    } catch {
      setStatus("La demande n’a pas pu être enregistrée.");
    }
  }

  async function remove(id: string) {
    await invoke("delete_memory", { id }).catch(() => undefined);
    await load();
  }

  async function schedule(id: string, time: string | null) {
    try {
      await invoke("set_routine_schedule", { id, schedule: time });
      setStatus(
        time
          ? `Planifiée chaque jour à ${time} (modèle local, lecture seule, AiWin11 doit être ouvert).`
          : "Planification retirée.",
      );
      await load();
    } catch {
      setStatus("Heure invalide.");
    }
  }

  return (
    <section aria-labelledby="routines-heading">
      <div className="section-header">
        <h3 id="routines-heading">Demandes enregistrées</h3>
        <button type="button" disabled={!prompt.trim()} onClick={() => void save()}>
          Enregistrer la demande
        </button>
      </div>
      {routines.length > 0 && (
        <ul>
          {routines.map((routine) => (
            <li key={routine.id}>
              <button type="button" className="link-button" onClick={() => onPick(routine.text)}>
                {routine.text}
              </button>
              <span>
                <input
                  type="time"
                  aria-label="Heure quotidienne"
                  value={times[routine.id] ?? routine.schedule ?? ""}
                  onChange={(event) => setTimes({ ...times, [routine.id]: event.target.value })}
                />
                <button
                  type="button"
                  disabled={!(times[routine.id] ?? routine.schedule)}
                  onClick={() => void schedule(routine.id, times[routine.id] ?? routine.schedule ?? null)}
                >
                  Planifier
                </button>
                {routine.schedule && (
                  <button type="button" onClick={() => void schedule(routine.id, null)}>
                    Déplanifier
                  </button>
                )}
                <button type="button" aria-label="Supprimer cette demande" onClick={() => void remove(routine.id)}>
                  Supprimer
                </button>
              </span>
            </li>
          ))}
        </ul>
      )}
      {reports.length > 0 && (
        <>
          <h3>Résultats des demandes planifiées</h3>
          <ul>
            {reports.map((report) => (
              <li key={report.id}>
                <span>
                  <time dateTime={report.createdAt}>{new Date(report.createdAt).toLocaleString("fr-FR")}</time> —{" "}
                  {report.text}
                </span>
                <button type="button" onClick={() => void remove(report.id)}>
                  Effacer
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
      <p role="status">{status}</p>
    </section>
  );
}
