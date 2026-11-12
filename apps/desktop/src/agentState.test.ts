import { describe, expect, it } from "vitest";
import { agentState, transferLabel } from "./agentState";

const base = { running: true, failed: false, paused: false, approvalPending: false, lastStage: "" };

describe("agent state", () => {
  it("is idle or error when nothing runs", () => {
    expect(agentState({ ...base, running: false })).toBe("idle");
    expect(agentState({ ...base, running: false, failed: true })).toBe("error");
  });

  it("follows the task stages", () => {
    expect(agentState({ ...base, lastStage: "task.created" })).toBe("analysing");
    expect(agentState({ ...base, lastStage: "task.planning" })).toBe("planning");
    expect(agentState({ ...base, lastStage: "agent.observation" })).toBe("planning");
    expect(agentState({ ...base, lastStage: "policy.approval.granted" })).toBe("executing");
    expect(agentState({ ...base, lastStage: "agent.tool.requested" })).toBe("executing");
    expect(agentState({ ...base, lastStage: "task.recovering" })).toBe("recovering");
  });

  it("puts pause first, then a pending approval", () => {
    expect(agentState({ ...base, approvalPending: true, lastStage: "agent.tool.requested" })).toBe("waiting");
    expect(agentState({ ...base, paused: true, approvalPending: true })).toBe("paused");
  });
});

describe("data transfer", () => {
  it("says whether the request left this PC", () => {
    expect(transferLabel(false, false, false)).toBeUndefined();
    expect(transferLabel(true, false, false)).toBe("Traitement sur ce PC");
    expect(transferLabel(true, true, true)).toBe("Données envoyées au cloud");
    expect(transferLabel(false, true, true)).toBe("Dernière demande envoyée au cloud");
    expect(transferLabel(false, false, true)).toBe("Dernière demande restée sur ce PC");
  });
});
