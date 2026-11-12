import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import "./overlay.css";

type OverlayStatus = { task: "idle" | "running" | "approval"; paletteShortcut: boolean; stopShortcut: boolean };
const destinations = [
  { target: "home", title: "Accueil" },
  { target: "dashboard", title: "Centre de contrôle" },
  { target: "models", title: "Modèles et routage" },
  { target: "security", title: "Sécurité et autorisations" },
  { target: "activity", title: "Activité" },
] as const;
const normalize = (text: string) =>
  text
    .toLocaleLowerCase("fr")
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "");

function Palette() {
  const input = useRef<HTMLInputElement>(null);
  const busy = useRef(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [status, setStatus] = useState<OverlayStatus>();
  const [error, setError] = useState("");
  const routes = destinations.map((route) => ({
    ...route,
    title:
      route.target === "home" && status?.task === "approval"
        ? "Revenir à l’approbation"
        : route.target === "home" && status?.task === "running"
          ? "Suivre la tâche"
          : route.title,
    draft: null,
  }));
  const term = query.trim();
  const draft = term && !term.startsWith(">") ? { target: "home", title: "Préparer la demande", draft: query } : null;
  const matches = routes.filter(({ title }) => normalize(title).includes(normalize(term.replace(/^>\s*/, ""))));
  const choices = draft ? [draft, ...matches] : matches;
  const active = Math.min(selected, choices.length - 1);

  useEffect(() => {
    let disposed = false;
    let frame = 0;
    const refresh = () => {
      void invoke<OverlayStatus>("overlay_status")
        .then((value) => {
          if (!disposed) setStatus(value);
        })
        .catch(() => undefined);
    };
    const opened = () => {
      if (disposed) return;
      input.current?.focus();
      refresh();
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        frame = requestAnimationFrame(() => void invoke("overlay_rendered").catch(() => undefined));
      });
    };
    const subscription = listen("desktop-palette-opened", opened);
    void subscription
      .then(() => {
        if (document.hasFocus()) opened();
      })
      .catch(() => undefined);
    const timer = window.setInterval(() => {
      if (document.hasFocus()) refresh();
    }, 600);
    window.addEventListener("focus", opened);
    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      clearInterval(timer);
      window.removeEventListener("focus", opened);
      void subscription.then((unlisten) => unlisten()).catch(() => undefined);
    };
  }, []);

  async function choose(choice: (typeof choices)[number]) {
    if (busy.current) return;
    busy.current = true;
    try {
      await invoke("overlay_open_workspace", { target: choice.target, draft: choice.draft });
      setQuery("");
      setSelected(0);
      setError("");
    } catch (code) {
      setError(
        code === "TASK_BUSY"
          ? "Une tâche est en cours. Le brouillon reste disponible ici."
          : "Impossible d’ouvrir l’espace de travail.",
      );
    } finally {
      busy.current = false;
    }
  }

  function dismiss() {
    void invoke("overlay_dismiss").catch(() => setError("Impossible de fermer la palette."));
  }

  return (
    <main
      className="palette"
      aria-labelledby="palette-title"
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          dismiss();
        }
      }}
    >
      <header>
        <h1 id="palette-title">Palette AiWin11</h1>
        <span className="task-state" data-task={status?.task} role="status">
          {status?.task === "approval"
            ? "Accord demandé"
            : status?.task === "running"
              ? "Tâche en cours"
              : status
                ? "Disponible"
                : ""}
        </span>
        <button type="button" className="close" aria-label="Fermer la palette" onClick={dismiss}>
          ×
        </button>
      </header>
      <label htmlFor="palette-query">Commande ou brouillon</label>
      <input
        ref={input}
        id="palette-query"
        placeholder="Écris une demande ou tape > pour filtrer"
        maxLength={2000}
        value={query}
        onChange={(event) => {
          setQuery(event.target.value);
          setSelected(0);
          setError("");
        }}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing || !choices.length) return;
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            setSelected((active + (event.key === "ArrowDown" ? 1 : -1) + choices.length) % choices.length);
          } else if (event.key === "Enter") {
            event.preventDefault();
            void choose(choices[active]);
          }
        }}
      />
      <ul aria-label="Commandes">
        {choices.map((choice, index) => (
          <li key={choice.draft === null ? choice.target : "draft"}>
            <button type="button" data-selected={index === active} onClick={() => void choose(choice)}>
              {choice.title}
              <span aria-hidden="true">↵</span>
            </button>
          </li>
        ))}
        {!choices.length && <li className="empty">Aucune commande trouvée.</li>}
      </ul>
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      <footer>
        <span>
          <kbd>↑</kbd> <kbd>↓</kbd> Choisir
        </span>
        <span>
          <kbd>Entrée</kbd> Ouvrir
        </span>
        <span>
          <kbd>Échap</kbd> Fermer
        </span>
      </footer>
      {status && (!status.paletteShortcut || !status.stopShortcut) && (
        <p className="warning">
          Raccourci indisponible ·{" "}
          {[!status.paletteShortcut && "Ctrl+Alt+Espace", !status.stopShortcut && "Ctrl+Alt+Retour arrière"]
            .filter(Boolean)
            .join(", ")}
        </p>
      )}
    </main>
  );
}

createRoot(document.getElementById("root")!).render(<Palette />);
