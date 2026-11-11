import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

const errors: Record<string, string> = {
  INVALID_FILE_NAME: "Indique un nom de fichier .txt sans chemin, avec des lettres non accentuées, des chiffres, des espaces, des tirets ou des underscores.",
  FILE_OUTSIDE_SCOPE: "Ce fichier est hors du périmètre autorisé.",
  FILE_TOO_LARGE: "La taille dépasse 64 Kio.",
  FILE_NOT_UTF8: "Enregistre le fichier au format texte UTF-8.",
  FILE_AUDIT_UNAVAILABLE: "Le journal local est indisponible.",
  FILE_DESTINATION_EXISTS: "Ce nom est déjà utilisé. Choisis un autre nom pour la copie.",
  FILE_COPY_FAILED: "La copie a échoué. Vérifie l’accès en écriture au dossier AiWin11.",
  FILE_COPY_CLEANUP_FAILED: "La copie a échoué et un fichier temporaire peut rester dans le dossier AiWin11.",
  CORE_BUSY: "AiWin11 traite déjà une autre opération. Réessaie dans un instant.",
};

export function LocalFiles({ onFinished }: { onFinished: () => void }) {
  const [root, setRoot] = useState("");
  const [name, setName] = useState("");
  const [destination, setDestination] = useState("");
  const [content, setContent] = useState<string>();
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void invoke<string>("local_files_root").then(setRoot, () => setMessage("Le dossier local est indisponible."));
  }, []);

  async function preview() {
    setContent(undefined);
    setMessage("Lecture en cours…");
    setBusy(true);
    try {
      setContent(await invoke<string>("preview_local_text", { name }));
      setMessage("");
    } catch (error) {
      setMessage(errors[String(error)] ?? "Vérifie que le fichier se trouve dans le dossier AiWin11 et qu’il est accessible.");
    } finally {
      setBusy(false);
      onFinished();
    }
  }

  async function copy() {
    setMessage("Copie en cours…");
    setBusy(true);
    try {
      await invoke("copy_local_text", { name, destination });
      setMessage(`Copie créée sous le nom « ${destination} ».`);
    } catch (error) {
      setMessage(errors[String(error)] ?? "Vérifie que le fichier source est accessible dans le dossier AiWin11.");
    } finally {
      setBusy(false);
      onFinished();
    }
  }

  return (
    <section className="card">
      {root && <p>Place le fichier texte dans ce dossier.<code className="file-path">{root}</code></p>}
      <form className="task-form" onSubmit={(event) => {
        event.preventDefault();
        if (root && name && !busy) void preview();
      }}>
        <label htmlFor="local-file-name">Nom du fichier texte</label>
        <input id="local-file-name" value={name} placeholder="notes.txt" maxLength={128}
          disabled={busy || !root} onChange={(event) => {
            setName(event.target.value);
            setContent(undefined);
            setMessage("");
          }} />
        <div><button disabled={busy || !root || !name}>Afficher l’aperçu</button></div>
      </form>
      <form className="task-form" onSubmit={(event) => {
        event.preventDefault();
        if (root && name && destination && !busy) void copy();
      }}>
        <label htmlFor="local-file-destination">Nom de la copie</label>
        <input id="local-file-destination" value={destination} placeholder="notes-copie.txt" maxLength={128}
          disabled={busy || !root} onChange={(event) => {
            setDestination(event.target.value);
            setMessage("");
          }} />
        <div><button disabled={busy || !root || !name || !destination}>Copier le fichier</button></div>
      </form>
      <p role="status">{message}</p>
      {content !== undefined && (
        <div>
          <button onClick={() => setContent(undefined)}>Effacer l’aperçu</button>
          <pre className="file-preview" aria-label="Contenu du fichier">{content || "Fichier vide."}</pre>
        </div>
      )}
    </section>
  );
}
