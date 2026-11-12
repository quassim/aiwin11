import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

/** File name patterns the agent may never read, list, copy, move or trash. */
export function ProtectedFiles() {
  const [saved, setSaved] = useState<string[]>();
  const [text, setText] = useState("");
  const [status, setStatus] = useState("");

  useEffect(() => {
    void invoke<string[]>("get_protected_files")
      .then((patterns) => {
        setSaved(patterns);
        setText(patterns.join("\n"));
      })
      .catch(() => setStatus("Règles indisponibles."));
  }, []);

  function save() {
    const patterns = text.split("\n");
    void invoke("set_protected_files", { patterns })
      .then(() => invoke<string[]>("get_protected_files"))
      .then((kept) => {
        setSaved(kept);
        setText(kept.join("\n"));
        setStatus(kept.length ? `${kept.length} règle(s) enregistrée(s).` : "Aucune règle : aucun fichier protégé.");
      })
      .catch(() => setStatus('Règles refusées : 20 au plus, 64 caractères chacune, sans \\ / : < > " |.'));
  }

  return (
    <section className="card" aria-labelledby="protected-heading">
      <h2 id="protected-heading">Fichiers protégés</h2>
      <p>
        Un modèle de nom par ligne, avec <code>*</code> pour n’importe quels caractères et <code>?</code> pour un seul
        (majuscules indifférentes), par exemple <code>contrat*</code>. L’agent ne voit pas ces fichiers et ne peut ni
        les lire, ni les copier, ni les renommer, ni les mettre à la corbeille, quel que soit le profil. Tu peux
        toujours les ouvrir toi-même dans la page Fichiers.
      </p>
      <label htmlFor="protected-files">Modèles de noms protégés</label>
      <textarea
        id="protected-files"
        rows={4}
        value={text}
        disabled={saved === undefined}
        onChange={(event) => setText(event.target.value)}
      />
      <div className="section-header">
        <button type="button" disabled={saved === undefined || text === saved.join("\n")} onClick={save}>
          Enregistrer les règles
        </button>
      </div>
      <p role="status">{status}</p>
    </section>
  );
}
