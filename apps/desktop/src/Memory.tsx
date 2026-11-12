import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type MemoryEntry = { id: string; createdAt: string; text: string };

/** Notes the agent was allowed to keep, encrypted on this PC. */
export function Memory() {
  const [notes, setNotes] = useState<MemoryEntry[]>();
  const [status, setStatus] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [embedding, setEmbedding] = useState("");

  async function load() {
    try {
      setNotes(await invoke<MemoryEntry[]>("get_memories"));
    } catch {
      setStatus("La mémoire est indisponible.");
    }
  }

  useEffect(() => {
    void load();
    void invoke<string>("get_embedding_model").then(setEmbedding, () => undefined);
  }, []);

  async function remove(id: string | null) {
    try {
      const removed = await invoke<number>("delete_memory", { id });
      setStatus(`${removed} information${removed > 1 ? "s" : ""} effacée${removed > 1 ? "s" : ""}.`);
    } catch {
      setStatus("L’effacement a échoué.");
    }
    setConfirming(false);
    await load();
  }

  return (
    <section className="card" aria-labelledby="memory-heading">
      <h2 id="memory-heading">Mémoire de l’agent</h2>
      <p>
        Ce que tu as demandé à l’agent de retenir. Chaque information est chiffrée pour ton compte Windows ; seuls les
        modèles locaux peuvent la relire.
      </p>
      {notes?.length ? (
        <ul>
          {notes.map((note) => (
            <li key={note.id}>
              <span>{note.text}</span>
              <button onClick={() => void remove(note.id)}>Effacer</button>
            </li>
          ))}
        </ul>
      ) : (
        notes && <p>Aucune information retenue.</p>
      )}
      {!!notes?.length &&
        (confirming ? (
          <span role="group" aria-label="Confirmer l’effacement">
            <button autoFocus onClick={() => setConfirming(false)}>
              Garder
            </button>{" "}
            <button className="danger" onClick={() => void remove(null)}>
              Tout effacer définitivement
            </button>
          </span>
        ) : (
          <button onClick={() => setConfirming(true)}>Tout effacer…</button>
        ))}
      <div className="task-form">
        <label htmlFor="embedding-model">Modèle d’embedding local pour la recherche (facultatif)</label>
        <input
          id="embedding-model"
          value={embedding}
          maxLength={256}
          placeholder="par exemple nomic-embed-text ; vide = recherche par mots-clés"
          onChange={(event) => setEmbedding(event.target.value)}
          onBlur={() => {
            void invoke("set_embedding_model", { model: embedding.trim() }).then(
              () => setStatus("Recherche mise à jour."),
              () => setStatus("Identifiant de modèle invalide."),
            );
          }}
        />
      </div>
      <p role="status">{status}</p>
    </section>
  );
}
