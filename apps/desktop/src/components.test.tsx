// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

// Every Tauri command is simulated: no native code runs in these tests.
const invoke = vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
}));

const { Memory } = await import("./Memory");
const { ModelConfig } = await import("./ModelConfig");
const { Routines } = await import("./Routines");
const { speak } = await import("./Speech");
const { Updates } = await import("./Updates");

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

describe("Updates", () => {
  it("shows unavailable message when updater key is not configured", async () => {
    invoke.mockResolvedValueOnce(false);
    render(<Updates running={false} />);
    expect(await screen.findByText("Canal de mises à jour signées indisponible dans cette version.")).toBeTruthy();
  });

  it("checks for updates and blocks install while a task is running", async () => {
    invoke.mockImplementation((command) => {
      if (command === "updates_configured") return Promise.resolve(true);
      if (command === "check_update") return Promise.resolve("0.2.0");
      if (command === "install_update") {
        const error = new Error("UPDATE_TASK_RUNNING");
        error.toString = () => "UPDATE_TASK_RUNNING";
        return Promise.reject(error);
      }
      return Promise.resolve(null);
    });
    const { rerender } = render(<Updates running={true} />);
    const checkButton = await screen.findByRole("button", { name: "Rechercher sur GitHub" });
    fireEvent.click(checkButton);
    expect(await screen.findByText("Version 0.2.0 disponible.")).toBeTruthy();
    const installButton = screen.getByRole<HTMLButtonElement>("button", {
      name: "Installer la version 0.2.0 et fermer AiWin11",
    });
    expect(installButton.disabled).toBe(true);

    rerender(<Updates running={false} />);
    expect(installButton.disabled).toBe(false);
    fireEvent.click(installButton);
    expect(await screen.findByText("Attends la fin de la tâche avant d’installer la mise à jour.")).toBeTruthy();
  });
});

describe("ModelConfig", () => {
  it("exports preferences and imports valid JSON configuration", async () => {
    invoke.mockResolvedValueOnce(undefined);
    const onImported = vi.fn();
    render(
      <ModelConfig
        preferences={{
          provider: "ollama",
          localModel: "qwen3:8b",
          cloudModel: "gpt-4o-mini",
          visionModel: "gemma3:4b",
        }}
        disabled={false}
        onImported={onImported}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Exporter dans le champ" }));
    const textarea = screen.getByLabelText<HTMLTextAreaElement>("Configuration des modèles (JSON)");
    expect(textarea.value).toContain('"provider": "ollama"');

    fireEvent.change(textarea, {
      target: {
        value: JSON.stringify({
          version: 1,
          provider: "openai",
          localModel: "qwen3:8b",
          cloudModel: "deepseek-chat",
          visionModel: "gemma3:4b",
        }),
      },
    });
    fireEvent.click(screen.getByRole("button", { name: "Appliquer cette configuration" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("set_model_preferences", {
        preferences: {
          provider: "openai",
          localModel: "qwen3:8b",
          cloudModel: "deepseek-chat",
          visionModel: "gemma3:4b",
        },
      }),
    );
    expect(onImported).toHaveBeenCalled();
  });
});
