// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

// Every Tauri command is simulated: no native code runs in these tests.
const invoke = vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
}));

const { Memory } = await import("./Memory");
const { Routines } = await import("./Routines");
const { speak } = await import("./Speech");

afterEach(() => {
  cleanup();
  invoke.mockReset();
});

describe("Memory page", () => {
  it("lists notes and deletes one after a click", async () => {
    invoke.mockImplementation((command) =>
      Promise.resolve(
        command === "get_memories"
          ? [{ id: "n1", createdAt: "2026-11-12T10:00:00Z", text: "Mon chat s'appelle Zéphyrin" }]
          : command === "delete_memory"
            ? 1
            : "",
      ),
    );
    render(<Memory />);
    expect(await screen.findByText("Mon chat s'appelle Zéphyrin")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Effacer" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("delete_memory", { id: "n1" }));
  });

  it("asks for a confirmation before erasing everything", async () => {
    invoke.mockImplementation((command) =>
      Promise.resolve(command === "get_memories" ? [{ id: "n1", createdAt: "", text: "Note" }] : ""),
    );
    render(<Memory />);
    fireEvent.click(await screen.findByRole("button", { name: "Tout effacer…" }));
    expect(screen.getByRole("button", { name: "Garder" })).toBeTruthy();
    expect(invoke).not.toHaveBeenCalledWith("delete_memory", { id: null });
  });
});

describe("Saved requests", () => {
  it("puts a saved request back in the field without sending it", async () => {
    invoke.mockImplementation((command) =>
      Promise.resolve(command === "get_routines" ? [{ id: "r1", createdAt: "", text: "Quel est l'état du PC ?" }] : []),
    );
    const onPick = vi.fn();
    render(<Routines prompt="" onPick={onPick} />);
    fireEvent.click(await screen.findByRole("button", { name: "Quel est l'état du PC ?" }));
    expect(onPick).toHaveBeenCalledWith("Quel est l'état du PC ?");
    expect(invoke).not.toHaveBeenCalledWith("run_task", expect.anything());
  });
});

describe("Speech", () => {
  it("never uses an online voice", () => {
    const spoken = vi.fn();
    const online = { localService: false, lang: "fr-FR", name: "Online" } as SpeechSynthesisVoice;
    Object.assign(window, {
      speechSynthesis: { getVoices: () => [online], cancel: vi.fn(), speak: spoken },
      SpeechSynthesisUtterance: class {
        constructor(public text: string) {}
      },
    });
    expect(speak("Bonjour")).toBe(false);
    expect(spoken).not.toHaveBeenCalled();
  });
});
