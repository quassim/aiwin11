import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type MemoryEntry = { id: string; createdAt: string; text: string };
type IndexedFile = { name: string; excerpt: string };

/** Retention choices for notes and reports, in days (0: until erased). */
const RETENTION_CHOICES: [number, string][] = [
  [0, "Jusqu’à ce que je les efface"],
  [30, "30 jours"],
  [90, "90 jours"],
  [365, "1 an"],
];

/** Notes the agent was allowed to keep, encrypted on this PC. */
export function Memory() {
  const [notes, setNotes] = useState<MemoryEntry[]>();
  const [status, setStatus] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [embedding, setEmbedding] = useState("");
  const [indexed, setIndexed] = useState<string[]>([]);
  const [filename, setFilename] = useState("");
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<IndexedFile[]>();
  const [busy, setBusy] = useState(false);
  const [retention, setRetention] = useState<number>();

  async function editIndex(name: string, remove: boolean) {
    setBusy(true);
    try {
      await invoke(remove ? "remove_indexed_file" : "index_local_text", { name });
      setIndexed(await invoke<string[]>("indexed_file_names"));
      setResults(undefined);
      setStatus(remove ? "Extrait effacé." : "Extrait indexé.");
    } catch (code) {
      setStatus(code === "MEMORY_FULL" ? "L’index contient déjà 32 fichiers." : "Le fichier n’a pas pu être traité.");
    } finally {
      setBusy(false);
    }
  }

  async function searchIndex() {
    setBusy(true);
    try {
      setResults(await invoke<IndexedFile[]>("search_indexed_files", { query }));
    } catch {
      setStatus("La recherche est indisponible.");
    } finally {
      setBusy(false);
    }
  }

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
    void invoke<number>("get_memory_retention").then(setRetention, () => undefined);
    void invoke<string[]>("indexed_file_names").then(setIndexed, () => setStatus("L’index est indisponible."));
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
        <label htmlFor="memory-retention">Conserver les informations et les rapports des demandes planifiées</label>
        <select
          id="memory-retention"
          value={retention ?? 0}
          disabled={retention === undefined}
          onChange={(event) => {
            const days = Number(event.target.value);
            void invoke<number>("set_memory_retention", { days }).then(
              (removed) => {
                setRetention(days);
                setStatus(
                  removed ? `${removed} élément(s) ancien(s) effacé(s).` : "Durée de conservation enregistrée.",
                );
                void load();
              },
              () => setStatus("La durée de conservation n’a pas pu être enregistrée."),
            );
          }}
        >
          {RETENTION_CHOICES.map(([days, label]) => (
            <option key={days} value={days}>
              {label}
            </option>
          ))}
        </select>
        <p>Les demandes enregistrées et l’index de fichiers ne sont jamais effacés automatiquement.</p>
      </div>
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
      <h3>Index local de fichiers</h3>
      <p>
        Ajoute un fichier texte du dossier de travail par son nom. Les 4 000 premiers caractères sont conservés sous
        forme chiffrée jusqu’à leur suppression. Réindexe le fichier après une modification.
      </p>
      <form
        className="task-form"
        onSubmit={(event) => {
          event.preventDefault();
          void editIndex(filename.trim(), false);
        }}
      >
        <label htmlFor="index-filename">Nom du fichier (.txt, .md ou .csv)</label>
        <input id="index-filename" value={filename} onChange={(event) => setFilename(event.target.value)} />
        <button disabled={busy || !filename.trim()}>Indexer le fichier</button>
      </form>
      {indexed.length > 0 && (
        <ul>
          {indexed.map((name) => (
            <li key={name}>
              {name}{" "}
              <button disabled={busy} onClick={() => void editIndex(name, true)}>
                Effacer l’extrait
              </button>
            </li>
          ))}
        </ul>
      )}
      <form
        className="task-form"
        onSubmit={(event) => {
          event.preventDefault();
          void searchIndex();
        }}
      >
        <label htmlFor="index-query">Rechercher dans les extraits</label>
        <input id="index-query" value={query} maxLength={1000} onChange={(event) => setQuery(event.target.value)} />
        <button disabled={busy || !query.trim() || !indexed.length}>Rechercher</button>
      </form>
      {results &&
        (results.length ? (
          results.map((result) => (
            <details key={result.name}>
              <summary>{result.name}</summary>
              <pre className="file-content">{result.excerpt}</pre>
            </details>
          ))
        ) : (
          <p>Aucun extrait correspondant.</p>
        ))}
      <p role="status">{status}</p>
    </section>
  );
}
