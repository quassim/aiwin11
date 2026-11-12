import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type Routine = { id: string; text: string };

/** Saved requests (encrypted): one click puts one back in the request field; it never runs by itself. */
export function Routines({ prompt, onPick }: { prompt: string; onPick: (text: string) => void }) {
  const [routines, setRoutines] = useState<Routine[]>([]);
  const [status, setStatus] = useState("");

  async function load() {
    try {
      setRoutines(await invoke<Routine[]>("get_routines"));
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
              <button type="button" aria-label="Supprimer cette demande" onClick={() => void remove(routine.id)}>
                Supprimer
              </button>
            </li>
          ))}
        </ul>
      )}
      <p role="status">{status}</p>
    </section>
  );
}
