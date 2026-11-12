import { invoke } from "@tauri-apps/api/core";
import { type ReactNode, useEffect, useState } from "react";

/** Editor for one list of protected name patterns (files or windows). */
function PatternRules({
  id,
  title,
  label,
  getCommand,
  setCommand,
  children,
}: {
  id: string;
  title: string;
  label: string;
  getCommand: string;
  setCommand: string;
  children: ReactNode;
}) {
  const [saved, setSaved] = useState<string[]>();
  const [text, setText] = useState("");
  const [status, setStatus] = useState("");

  useEffect(() => {
    void invoke<string[]>(getCommand)
      .then((patterns) => {
        setSaved(patterns);
        setText(patterns.join("\n"));
      })
      .catch(() => setStatus("Règles indisponibles."));
  }, [getCommand]);

  function save() {
    const patterns = text.split("\n");
    void invoke(setCommand, { patterns })
      .then(() => invoke<string[]>(getCommand))
      .then((kept) => {
        setSaved(kept);
        setText(kept.join("\n"));
        setStatus(kept.length ? `${kept.length} règle(s) enregistrée(s).` : "Aucune règle enregistrée.");
      })
      .catch(() => setStatus('Règles refusées : 20 au plus, 64 caractères chacune, sans \\ / : < > " |.'));
  }

  return (
    <section className="card" aria-labelledby={`${id}-heading`}>
      <h2 id={`${id}-heading`}>{title}</h2>
      {children}
      <label htmlFor={id}>{label}</label>
      <textarea
        id={id}
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

const wildcards = (
  <>
    Un modèle par ligne, avec <code>*</code> pour n’importe quels caractères et <code>?</code> pour un seul (majuscules
    indifférentes).
  </>
);

/** File name patterns the agent may never read, list, copy, move or trash. */
export function ProtectedFiles() {
  return (
    <PatternRules
      id="protected-files"
      title="Fichiers protégés"
      label="Modèles de noms protégés"
      getCommand="get_protected_files"
      setCommand="set_protected_files"
    >
      <p>
        {wildcards} Par exemple <code>contrat*</code>. L’agent ne voit pas ces fichiers et ne peut ni les lire, ni les
        copier, ni les renommer, ni les mettre à la corbeille, quel que soit le profil. Tu peux toujours les ouvrir
        toi-même dans la page Fichiers.
      </p>
    </PatternRules>
  );
}

/** Window patterns (title or application) the agent may never see or act on. */
export function ProtectedWindows() {
  return (
    <PatternRules
      id="protected-windows"
      title="Fenêtres protégées"
      label="Modèles de titres ou d’applications protégés"
      getCommand="get_protected_windows"
      setCommand="set_protected_windows"
    >
      <p>
        {wildcards} Chaque modèle est comparé au titre de la fenêtre et au nom de son application, par exemple{" "}
        <code>*Bitwarden*</code> ou <code>keepass*</code>. L’agent ne voit pas ces fenêtres dans la liste et ne peut ni
        les lire, ni les capturer, ni les organiser, ni y cliquer, quel que soit le profil.
      </p>
    </PatternRules>
  );
}
