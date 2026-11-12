import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type McpServer = { name: string; command: string; args: string[]; enabled: boolean; allowedTools: string[] | null };
type Draft = Omit<McpServer, "args" | "allowedTools"> & { args: string; tools: string; restrict: boolean };

const toDraft = (server: McpServer): Draft => ({
  ...server,
  args: server.args.join("\n"),
  tools: server.allowedTools?.join("\n") ?? "",
  restrict: server.allowedTools != null,
});
const lines = (text: string) =>
  text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);

/** MCP servers the user declares (stdio, fixed executable and arguments, no shell). */
export function Extensions() {
  const [drafts, setDrafts] = useState<Draft[]>([]);
  const [status, setStatus] = useState("");

  useEffect(() => {
    void invoke<McpServer[]>("get_mcp_servers").then(
      (servers) => setDrafts(servers.map(toDraft)),
      () => setStatus("Les extensions sont indisponibles."),
    );
  }, []);

  const update = (index: number, change: Partial<Draft>) =>
    setDrafts(drafts.map((draft, i) => (i === index ? { ...draft, ...change } : draft)));

  async function save() {
    const servers: McpServer[] = drafts.map((draft) => ({
      name: draft.name.trim(),
      command: draft.command.trim(),
      args: lines(draft.args),
      enabled: draft.enabled,
      allowedTools: draft.restrict ? lines(draft.tools) : null,
    }));
    try {
      await invoke("set_mcp_servers", { servers });
      setStatus("Extensions enregistrées.");
    } catch {
      setStatus(
        "Vérifie chaque extension : nom court (lettres, chiffres, - ou _), chemin complet d’un exécutable existant, 16 arguments au plus, noms uniques.",
      );
    }
  }

  return (
    <section className="card" aria-labelledby="extensions-heading">
      <h2 id="extensions-heading">Extensions MCP</h2>
      <p>
        Déclare des serveurs MCP locaux (stdio). L’agent peut lister leurs outils et, avec ton accord à chaque fois, en
        appeler un. Un serveur MCP est un programme que tu choisis d’exécuter : n’ajoute que des extensions de
        confiance. Seuls les modèles locaux y ont accès.
      </p>
      {drafts.map((draft, index) => (
        <fieldset key={index} className="task-form">
          <legend>Extension {index + 1}</legend>
          <label>
            <input
              type="checkbox"
              checked={draft.enabled}
              onChange={(event) => update(index, { enabled: event.target.checked })}
            />{" "}
            Activer ce serveur
          </label>
          <label htmlFor={`mcp-name-${index}`}>Nom</label>
          <input
            id={`mcp-name-${index}`}
            value={draft.name}
            maxLength={32}
            onChange={(event) => update(index, { name: event.target.value })}
          />
          <label htmlFor={`mcp-command-${index}`}>Exécutable (chemin complet)</label>
          <input
            id={`mcp-command-${index}`}
            value={draft.command}
            onChange={(event) => update(index, { command: event.target.value })}
          />
          <label htmlFor={`mcp-args-${index}`}>Arguments (un par ligne)</label>
          <textarea
            id={`mcp-args-${index}`}
            rows={3}
            value={draft.args}
            onChange={(event) => update(index, { args: event.target.value })}
          />
          <label>
            <input
              type="checkbox"
              checked={draft.restrict}
              onChange={(event) => update(index, { restrict: event.target.checked })}
            />{" "}
            Limiter les outils autorisés
          </label>
          {draft.restrict && (
            <>
              <label htmlFor={`mcp-tools-${index}`}>Outils autorisés (un identifiant par ligne)</label>
              <textarea
                id={`mcp-tools-${index}`}
                rows={3}
                value={draft.tools}
                onChange={(event) => update(index, { tools: event.target.value })}
              />
            </>
          )}
          <button type="button" onClick={() => setDrafts(drafts.filter((_, i) => i !== index))}>
            Retirer
          </button>
        </fieldset>
      ))}
      <div className="section-header">
        <button
          type="button"
          disabled={drafts.length >= 5}
          onClick={() =>
            setDrafts([...drafts, { name: "", command: "", args: "", enabled: false, restrict: true, tools: "" }])
          }
        >
          Ajouter une extension
        </button>
        <button type="button" onClick={() => void save()}>
          Enregistrer
        </button>
      </div>
      <p role="status">{status}</p>
    </section>
  );
}
