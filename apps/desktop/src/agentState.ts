/** Agent state always shown in the sidebar ([S0 §6]: idle, analysing, planning, waiting, executing, error). */
export type AgentState =
  "idle" | "analysing" | "planning" | "waiting" | "executing" | "paused" | "recovering" | "error";

export const agentStateLabels: Record<AgentState, string> = {
  idle: "Inactif",
  analysing: "Analyse de la demande",
  planning: "Planification",
  waiting: "En attente de ton approbation",
  executing: "Exécution d’une action",
  paused: "En pause",
  recovering: "Nouvel essai après un échec passager",
  error: "Erreur sur la dernière demande",
};

// The model is choosing the next step.
// Stages sent by the core (`TaskUpdate::stage`); after an observation or a decision the model plans again.
const planningStages = new Set([
  "task.planning",
  "task.resumed",
  "agent.observation",
  "agent.reply",
  "policy.approval.declined",
  "policy.approval.expired",
  "policy.simulated",
]);
// An approved or allowed tool call is being carried out.
const executingStages = new Set(["agent.tool.requested", "policy.approval.granted", "workflow.step.started"]);

export function agentState(input: {
  running: boolean;
  failed: boolean;
  paused: boolean;
  approvalPending: boolean;
  lastStage: string;
}): AgentState {
  if (!input.running) return input.failed ? "error" : "idle";
  if (input.paused) return "paused";
  if (input.approvalPending) return "waiting";
  if (input.lastStage === "task.recovering") return "recovering";
  if (executingStages.has(input.lastStage)) return "executing";
  if (planningStages.has(input.lastStage)) return "planning";
  return "analysing";
}
