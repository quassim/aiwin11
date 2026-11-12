import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type Routine = { id: string; text: string; createdAt: string; schedule?: string };
type WorkflowStep = { text: string; onFailure: boolean; branches?: [number, number] };
const workflowHeader = "Exécute ces étapes dans l’ordre. Si une étape échoue, arrête-toi et explique pourquoi.";
const failurePrefix = "[si échec] ";
const maxRoutineChars = 1000;
const everyDay = "0123456";
const weekdays = [
  ["1", "Lun."],
  ["2", "Mar."],
  ["3", "Mer."],
  ["4", "Jeu."],
  ["5", "Ven."],
  ["6", "Sam."],
  ["0", "Dim."],
] as const;

function formatWorkflow(steps: WorkflowStep[]) {
  return [
    workflowHeader,
    ...steps.map(
      (step, index) =>
        `${index + 1}. ${step.onFailure ? failurePrefix : ""}${step.branches ? `[suite:${step.branches.join("/")}] ` : ""}${step.text.trim()}`,
    ),
  ].join("\n");
}

/**
 * Saved requests (encrypted). One click puts one back in the request field. A daily time makes
 * AiWin11 run it alone with the local model in the Observation profile (reads only).
 */
export function Routines({ prompt, onPick }: { prompt: string; onPick: (text: string) => void }) {
  const [routines, setRoutines] = useState<Routine[]>([]);
  const [reports, setReports] = useState<Routine[]>([]);
  const [times, setTimes] = useState<Record<string, string>>({});
  const [days, setDays] = useState<Record<string, string>>({});
  const [status, setStatus] = useState("");
  const [steps, setSteps] = useState<WorkflowStep[]>();
  const [editing, setEditing] = useState<string>();
  const [saving, setSaving] = useState(false);
  const workflowText = steps ? formatWorkflow(steps) : "";
  const workflowLength = Array.from(workflowText).length;
  const canSaveWorkflow =
    steps?.every(
      (step, index) =>
        (index > 0 || !step.onFailure) &&
        step.text.trim() &&
        !step.text.trim().startsWith(failurePrefix) &&
        !step.text.trim().startsWith("[suite:") &&
        !/[\r\n\0]/.test(step.text),
    ) && workflowLength <= maxRoutineChars;

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

  function editWorkflow(routine: Routine) {
    setEditing(routine.id);
    setSteps(
      routine.text
        .split("\n")
        .slice(1)
        .map((line) => {
          const text = line.replace(/^\d+\. /, "");
          const onFailure = text.startsWith(failurePrefix);
          const body = onFailure ? text.slice(failurePrefix.length) : text;
          const route = /^\[suite:(\d+)\/(\d+)\] (.*)$/.exec(body);
          return route
            ? { text: route[3], onFailure, branches: [Number(route[1]), Number(route[2])] }
            : { text: body, onFailure };
        }),
    );
    setStatus("");
  }

  async function saveWorkflow() {
    if (!canSaveWorkflow || saving) return;
    setSaving(true);
    try {
      if (editing) {
        await invoke("update_routine", { id: editing, text: workflowText });
      } else {
        await invoke("save_routine", { text: workflowText });
      }
      setSteps(undefined);
      setEditing(undefined);
      setStatus("Workflow enregistré.");
      await load();
    } catch {
      setStatus("Le workflow n’a pas pu être enregistré.");
    } finally {
      setSaving(false);
    }
  }

  function moveStep(index: number, offset: number) {
    setSteps((current) => {
      if (!current) return current;
      const next = [...current];
      const target = index + offset;
      [next[index], next[target]] = [next[target], next[index]];
      return next;
    });
  }

  function updateStep(index: number, change: Partial<WorkflowStep>) {
    setSteps((current) => current?.map((step, position) => (position === index ? { ...step, ...change } : step)));
  }

  function removeStep(index: number) {
    const adjust = (target: number) => (target === index + 1 ? 0 : target > index + 1 ? target - 1 : target);
    setSteps((current) =>
      current
        ?.filter((_, position) => position !== index)
        .map((step, position) => ({
          ...step,
          onFailure: position > 0 && step.onFailure,
          branches: step.branches && [adjust(step.branches[0]), adjust(step.branches[1])],
        })),
    );
  }

  async function remove(id: string) {
    await invoke("delete_memory", { id }).catch(() => undefined);
    await load();
  }

  async function schedule(id: string, time: string | null, selected = everyDay) {
    try {
      const value = time && time !== "startup" && selected !== everyDay ? `${time}@${selected}` : time;
      await invoke("set_routine_schedule", { id, schedule: value });
      setStatus(
        time === "startup"
          ? "Au prochain démarrage d’AiWin11, au plus une fois par jour (modèle local, lecture seule)."
          : time
            ? `Planifiée à ${time} les jours sélectionnés (modèle local, lecture seule, AiWin11 doit être ouvert).`
            : "Planification retirée.",
      );
      await load();
    } catch {
      setStatus("Impossible d’enregistrer la planification. Vérifie l’heure et les jours choisis.");
    }
  }

  return (
    <section aria-labelledby="routines-heading">
      <div className="section-header">
        <h3 id="routines-heading">Demandes enregistrées</h3>
        <button type="button" disabled={!prompt.trim()} onClick={() => void save()}>
          Enregistrer la demande
        </button>
        <button
          type="button"
          disabled={!!steps}
          onClick={() => {
            setEditing(undefined);
            setSteps([{ text: "", onFailure: false }]);
          }}
        >
          Créer un workflow
        </button>
      </div>
      {steps && (
        <fieldset className="task-form" disabled={saving}>
          <legend>{editing ? "Modifier le workflow" : "Nouveau workflow"}</legend>
          <p>
            Les étapes s’exécutent avec Ollama. Un branchement direct peut mener à une étape suivante ou terminer le
            parcours. Un refus d’autorisation arrête le workflow.
          </p>
          <ol className="workflow-nodes" aria-label="Parcours du workflow">
            {steps.map((step, index) => (
              <li key={index} className="workflow-node">
                <p className="workflow-transition">
                  {index === 0
                    ? "Départ"
                    : step.onFailure
                      ? "Après un échec ou un branchement direct"
                      : "Après une réussite ou un branchement direct"}
                </p>
                <div className="task-form workflow-step">
                  <label htmlFor={`workflow-step-${index}`}>Étape {index + 1}</label>
                  <input
                    id={`workflow-step-${index}`}
                    value={step.text}
                    maxLength={maxRoutineChars}
                    onChange={(event) =>
                      setSteps(
                        steps.map((value, position) =>
                          position === index ? { ...value, text: event.target.value } : value,
                        ),
                      )
                    }
                  />
                  {(index > 0 || step.onFailure) && (
                    <label className="approval">
                      <input
                        type="checkbox"
                        checked={step.onFailure}
                        disabled={index === 0 && !step.onFailure}
                        onChange={(event) =>
                          setSteps(
                            steps.map((value, position) =>
                              position === index ? { ...value, onFailure: event.target.checked } : value,
                            ),
                          )
                        }
                      />
                      <span>Si échec de la dernière étape exécutée</span>
                    </label>
                  )}
                  <label className="approval">
                    <input
                      type="checkbox"
                      checked={!!step.branches}
                      onChange={(event) =>
                        updateStep(index, {
                          branches: event.target.checked ? [index + 1 < steps.length ? index + 2 : 0, 0] : undefined,
                        })
                      }
                    />
                    <span>Choisir les étapes suivantes</span>
                  </label>
                  {step.branches &&
                    ([0, 1] as const).map((branch) => (
                      <label key={branch}>
                        {branch === 0 ? "En cas de réussite" : "En cas d’échec"}
                        <select
                          value={step.branches?.[branch]}
                          onChange={(event) => {
                            const target = Number(event.target.value);
                            const [success, failure] = step.branches!;
                            updateStep(index, { branches: branch === 0 ? [target, failure] : [success, target] });
                          }}
                        >
                          <option value={0}>Terminer</option>
                          {steps.slice(index + 1).map((_, relative) => (
                            <option key={relative} value={index + relative + 2}>
                              Étape {index + relative + 2}
                            </option>
                          ))}
                        </select>
                      </label>
                    ))}
                  <div>
                    <button
                      type="button"
                      disabled={index === 0 || steps.some((step) => step.branches)}
                      onClick={() => moveStep(index, -1)}
                    >
                      Monter
                    </button>
                    <button
                      type="button"
                      disabled={index === steps.length - 1 || steps.some((step) => step.branches)}
                      onClick={() => moveStep(index, 1)}
                    >
                      Descendre
                    </button>
                    {editing && index > 0 && (
                      <button
                        type="button"
                        disabled={!canSaveWorkflow || step.onFailure}
                        onClick={() => {
                          const rebase = (target: number) => (target === 0 ? 0 : target - index);
                          onPick(
                            formatWorkflow(
                              steps.slice(index).map((step) => ({
                                ...step,
                                branches: step.branches && [rebase(step.branches[0]), rebase(step.branches[1])],
                              })),
                            ),
                          );
                          setStatus(
                            `Reprise préparée depuis l’étape ${index + 1}. Vérifie la demande avant de l’envoyer.`,
                          );
                        }}
                      >
                        Préparer la reprise ici
                      </button>
                    )}
                    <button type="button" disabled={steps.length === 1} onClick={() => removeStep(index)}>
                      Retirer
                    </button>
                  </div>
                </div>
              </li>
            ))}
          </ol>
          <p>
            {workflowLength} / {maxRoutineChars} caractères
          </p>
          <div className="section-header">
            <button
              type="button"
              disabled={steps.length >= 8}
              onClick={() => setSteps([...steps, { text: "", onFailure: false }])}
            >
              Ajouter une étape
            </button>
            <button type="button" disabled={!canSaveWorkflow} onClick={() => void saveWorkflow()}>
              Enregistrer le workflow
            </button>
            <button
              type="button"
              onClick={() => {
                setSteps(undefined);
                setEditing(undefined);
              }}
            >
              Annuler
            </button>
          </div>
        </fieldset>
      )}
      {routines.length > 0 && (
        <ul>
          {routines.map((routine) => {
            const atStartup = routine.schedule === "startup";
            const [savedTime, savedDays = everyDay] = (atStartup ? "" : (routine.schedule ?? "")).split("@");
            const time = times[routine.id] ?? savedTime;
            const selected = days[routine.id] ?? savedDays;
            return (
              <li key={routine.id}>
                <button type="button" className="link-button" onClick={() => onPick(routine.text)}>
                  {routine.text.startsWith(workflowHeader + "\n")
                    ? routine.text.split("\n").slice(1).join(" · ")
                    : routine.text}
                </button>
                <div>
                  {routine.text.startsWith(workflowHeader + "\n") && (
                    <button type="button" disabled={!!steps} onClick={() => editWorkflow(routine)}>
                      Modifier
                    </button>
                  )}
                  <input
                    type="time"
                    aria-label="Heure locale d’exécution"
                    value={time}
                    onChange={(event) => setTimes({ ...times, [routine.id]: event.target.value })}
                  />
                  <fieldset>
                    <legend>Jours d’exécution</legend>
                    {weekdays.map(([day, label]) => (
                      <label key={day}>
                        <input
                          type="checkbox"
                          checked={selected.includes(day)}
                          onChange={(event) => {
                            const next = event.target.checked
                              ? Array.from(selected + day)
                                  .sort()
                                  .join("")
                              : selected.replace(day, "");
                            setDays((current) => ({ ...current, [routine.id]: next }));
                          }}
                        />
                        {label}
                      </label>
                    ))}
                  </fieldset>
                  <button
                    type="button"
                    disabled={!time || !selected}
                    onClick={() => void schedule(routine.id, time, selected)}
                  >
                    Planifier
                  </button>
                  <button type="button" disabled={atStartup} onClick={() => void schedule(routine.id, "startup")}>
                    Au démarrage
                  </button>
                  {atStartup && <p>Prévu au démarrage d’AiWin11.</p>}
                  {routine.schedule && (
                    <button type="button" onClick={() => void schedule(routine.id, null)}>
                      Déplanifier
                    </button>
                  )}
                  <button type="button" aria-label="Supprimer cette demande" onClick={() => void remove(routine.id)}>
                    Supprimer
                  </button>
                </div>
              </li>
            );
          })}
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
