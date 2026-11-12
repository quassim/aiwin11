import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type Overview = { saved: string[]; aliases: string[] };

/** Up to four more folders the agent may work in, reached as `alias/name.txt`. */
export function ExtraFolders() {
  const [overview, setOverview] = useState<Overview>();
  const [text, setText] = useState("");
  const [status, setStatus] = useState("");

  useEffect(() => {
    void invoke<Overview>("get_extra_folders")
      .then((next) => {
        setOverview(next);
        setText(next.saved.join("\n"));
      })
      .catch(() => setStatus("Réglage indisponible."));
  }, []);

  function save() {
    const paths = text.split("\n");
    void invoke<string[]>("set_extra_folders", { paths })
      .then((stored) => {
        setText(stored.join("\n"));
        setOverview((current) => current && { ...current, saved: stored });
        setStatus(
          stored.length
            ? `${stored.length} dossier(s) enregistré(s), utilisés au prochain démarrage d’AiWin11.`
            : "Aucun dossier supplémentaire au prochain démarrage.",
        );
      })
      .catch((error) =>
        setStatus(
          String(error) === "SETTINGS_INVALID"
            ? "Quatre dossiers au plus."
            : "Un des dossiers est refusé : il doit exister, être dans ton profil Windows et hors AppData.",
        ),
      );
  }

  return (
    <div className="task-form">
      <label htmlFor="extra-folders">Dossiers supplémentaires de l’agent (un chemin par ligne, quatre au plus)</label>
      <textarea
        id="extra-folders"
        rows={3}
        value={text}
        disabled={overview === undefined}
        onChange={(event) => setText(event.target.value)}
      />
      <p>
        L’agent y accède avec le nom court du dossier, par exemple <code>documents/notes.txt</code>. Une copie ou un
        renommage reste toujours dans le même dossier, et les mêmes règles s’appliquent (formats texte, fichiers
        protégés, accord selon le profil).
        {overview && overview.aliases.length > 0 && <> En service : {overview.aliases.join(", ")}.</>}
      </p>
      <div className="section-header">
        <button type="button" disabled={overview === undefined} onClick={save}>
          Enregistrer les dossiers
        </button>
      </div>
      <p role="status">{status}</p>
    </div>
  );
}
