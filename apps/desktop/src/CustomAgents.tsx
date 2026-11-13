import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

export type CustomAgent = {
  name: string;
  base: string;
  role: string;
  tools: string[];
  model: string;
  maxSteps: number;
};
type ToolOptions = { base: string; tools: string[] }[];

export const baseLabels: Record<string, string> = {
  office: "Fichiers et bureautique",
  windowsAdmin: "Administration Windows",
  development: "Développement",
  research: "Recherche",
  multimedia: "Multimédia",
  readOnly: "Lecture seule",
};

const empty = (): CustomAgent => ({ name: "", base: "readOnly", role: "", tools: [], model: "", maxSteps: 8 });

/**
 * Agents the user defines as data: a role, a built-in agent they narrow, an optional subset of
 * its tools, a local model and a step limit. They never get more rights than their base.
 */
export function CustomAgents({ onSaved }: { onSaved: (agents: CustomAgent[]) => void }) {
  const [agents, setAgents] = useState<CustomAgent[]>();
  const [options, setOptions] = useState<ToolOptions>([]);
  const [status, setStatus] = useState("");

  useEffect(() => {
    void invoke<CustomAgent[]>("get_custom_agents").then(setAgents, () => setStatus("Agents indisponibles."));
    void invoke<ToolOptions>("agent_tool_options").then(setOptions, () => setOptions([]));
  }, []);

  const update = (index: number, change: Partial<CustomAgent>) =>
    setAgents((current) => current?.map((agent, position) => (position === index ? { ...agent, ...change } : agent)));

  async function save() {
    if (!agents) return;
    const cleaned = agents.map((agent) => ({ ...agent, name: agent.name.trim(), model: agent.model.trim() }));
    try {
      await invoke("set_custom_agents", { agents: cleaned });
      setStatus("Agents enregistrés.");
      onSaved(cleaned);
    } catch {
      setStatus(
        "Vérifie chaque agent : nom court en minuscules (lettres, chiffres, -), unique ; rôle de 600 caractères au plus ; 1 à 30 étapes.",
      );
    }
  }

  return (
    <section className="card" aria-labelledby="custom-agents-heading">
      <h2 id="custom-agents-heading">Agents personnalisés</h2>
      <p>
        Un agent personnalisé part d’un agent spécialisé et ne peut que le restreindre : il n’utilise que les outils
        cochés (tous ceux de sa base si aucun) et chaque action suit les mêmes règles et accords. L’agent général peut
        lui déléguer une sous-demande ; il ne lui transmet aucune autorisation.
      </p>
      {agents?.map((agent, index) => {
        const tools = options.find((option) => option.base === agent.base)?.tools ?? [];
        return (
          <fieldset key={index} className="task-form">
            <legend>{agent.name || `Agent ${index + 1}`}</legend>
            <label htmlFor={`custom-agent-name-${index}`}>Nom (utilisé comme perso:nom)</label>
            <input
              id={`custom-agent-name-${index}`}
              value={agent.name}
              maxLength={24}
              onChange={(event) => update(index, { name: event.target.value.toLowerCase() })}
            />
            <label htmlFor={`custom-agent-base-${index}`}>Agent de base</label>
            <select
              id={`custom-agent-base-${index}`}
              value={agent.base}
              onChange={(event) => update(index, { base: event.target.value, tools: [] })}
            >
              {Object.entries(baseLabels).map(([base, label]) => (
                <option key={base} value={base}>
                  {label}
                </option>
              ))}
            </select>
            <label htmlFor={`custom-agent-role-${index}`}>Rôle (instructions pour le modèle)</label>
            <textarea
              id={`custom-agent-role-${index}`}
              rows={3}
              maxLength={600}
              value={agent.role}
              onChange={(event) => update(index, { role: event.target.value })}
            />
            <fieldset>
              <legend>Outils autorisés (aucun coché : tous ceux de l’agent de base)</legend>
              {tools.map((tool) => (
                <label key={tool} className="approval">
                  <input
                    type="checkbox"
                    checked={agent.tools.includes(tool)}
                    onChange={(event) =>
                      update(index, {
                        tools: event.target.checked
                          ? [...agent.tools, tool]
                          : agent.tools.filter((value) => value !== tool),
                      })
                    }
                  />
                  <code>{tool}</code>
                </label>
              ))}
            </fieldset>
            <label htmlFor={`custom-agent-model-${index}`}>Modèle Ollama (facultatif)</label>
            <input
              id={`custom-agent-model-${index}`}
              value={agent.model}
              maxLength={256}
              placeholder="vide : modèle de la demande"
              onChange={(event) => update(index, { model: event.target.value })}
            />
            <label htmlFor={`custom-agent-steps-${index}`}>Étapes au plus</label>
            <input
              id={`custom-agent-steps-${index}`}
              type="number"
              min={1}
              max={30}
              value={agent.maxSteps}
              onChange={(event) => update(index, { maxSteps: Number(event.target.value) })}
            />
            <button type="button" onClick={() => setAgents(agents.filter((_, position) => position !== index))}>
              Retirer
            </button>
          </fieldset>
        );
      })}
      <div className="section-header">
        <button
          type="button"
          disabled={!agents || agents.length >= 8}
          onClick={() => setAgents([...(agents ?? []), empty()])}
        >
          Ajouter un agent
        </button>
        <button type="button" disabled={!agents} onClick={() => void save()}>
          Enregistrer les agents
        </button>
      </div>
      <p role="status">{status}</p>
    </section>
  );
}
