import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type Node = { parent: number; controlType: number; enabled: boolean; focused: boolean };
type Inspection = { nodes: Node[]; truncated: boolean };
type Capture = { jpeg: number[]; width: number; height: number };
type Display = { x: number; y: number; width: number; height: number; primary: boolean };
type Action = "inspect" | "capture" | "display" | "input";
const actionLabels: Record<Action, string> = {
  inspect: "Inspection",
  capture: "Capture",
  display: "Capture de l’écran entier",
  input: "Saisie",
};
const actionErrors: Record<Action, string> = {
  inspect: "Impossible d’inspecter cette fenêtre. Réessaie en la gardant au premier plan.",
  capture: "Capture indisponible pour cette fenêtre. Garde-la au premier plan et réessaie.",
  display: "Impossible de capturer cet écran. Actualise la liste des écrans et réessaie.",
  input: "La saisie n’a pas pu être confirmée. Vérifie la Table des caractères avant de réessayer.",
};

const roles: Record<number, string> = {
  50000: "Bouton",
  50001: "Calendrier",
  50002: "Case à cocher",
  50003: "Liste déroulante",
  50004: "Champ de saisie",
  50005: "Lien",
  50006: "Image",
  50007: "Élément de liste",
  50008: "Liste",
  50009: "Menu",
  50010: "Barre de menus",
  50011: "Élément de menu",
  50013: "Bouton radio",
  50014: "Barre de défilement",
  50017: "Onglets",
  50018: "Onglet",
  50020: "Texte",
  50021: "Barre d’outils",
  50022: "Info-bulle",
  50023: "Arborescence",
  50024: "Nœud",
  50025: "Contrôle personnalisé",
  50026: "Groupe",
  50028: "Grille",
  50029: "Cellule",
  50030: "Document",
  50031: "Bouton scindé",
  50032: "Fenêtre",
  50033: "Volet",
  50034: "En-tête",
  50036: "Tableau",
  50037: "Barre de titre",
};

export function Inspector({ onFinished }: { onFinished: () => void }) {
  const [count, setCount] = useState(0);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Inspection>();
  const [capture, setCapture] = useState<Capture>();
  const [action, setAction] = useState<Action>("inspect");
  const [digit, setDigit] = useState("");
  const [message, setMessage] = useState("");
  const [displays, setDisplays] = useState<Display[]>();
  const [selectedDisplay, setSelectedDisplay] = useState<Display>();

  async function listDisplays() {
    setBusy(true);
    try {
      setDisplays(await invoke<Display[]>("list_displays"));
      setMessage("");
    } catch {
      setDisplays(undefined);
      setMessage("Impossible de lire la disposition des écrans.");
    } finally {
      setBusy(false);
      onFinished();
    }
  }

  useEffect(() => {
    if (count === 0) return;
    const timer = window.setTimeout(() => {
      setCount(count - 1);
      if (count !== 1) return;
      setBusy(true);
      setMessage(`${actionLabels[action]} en cours…`);
      const request =
        action === "input"
          ? invoke<boolean>("enter_character_map_digit", { digit: Number(digit) }).then((verified) => {
              setMessage(
                verified
                  ? "Chiffre saisi. Le champ de la Table des caractères contient le chiffre."
                  : "Saisie envoyée. La valeur du champ n’a pas pu être confirmée.",
              );
            })
          : action === "capture" || action === "display"
            ? (action === "display"
                ? invoke<Capture>("capture_display", { display: selectedDisplay })
                : invoke<Capture>("capture_foreground")
              ).then((frame) => {
                setCapture(frame);
                setMessage("");
              })
            : invoke<Inspection>("inspect_foreground").then((tree) => {
                setResult(tree);
                setMessage(tree.truncated ? "Inspection partielle. La limite de parcours a été atteinte." : "");
              });
      void request
        .catch((error) =>
          setMessage(
            error === "CHARACTER_MAP_UNAVAILABLE"
              ? "Place la Table des caractères au premier plan et vérifie que son champ est accessible."
              : actionErrors[action],
          ),
        )
        .finally(() => {
          setBusy(false);
          onFinished();
        });
    }, 1000);
    return () => window.clearTimeout(timer);
  }, [action, count, digit, onFinished, selectedDisplay]);

  function start(nextAction: Action) {
    setResult(undefined);
    setCapture(undefined);
    setMessage("");
    setAction(nextAction);
    setCount(3);
  }

  function renderChildren(parent: number) {
    return result?.nodes
      .map((node, index) => ({ node, index }))
      .filter(({ node }) => node.parent === parent)
      .map(({ node, index }): React.ReactNode => (
        <li key={index}>
          {roles[node.controlType] ?? `Contrôle ${node.controlType}`}
          {node.focused && " · Focus"}
          {!node.enabled && " · Désactivé"}
          {result.nodes.some((child) => child.parent === index + 1) && <ul>{renderChildren(index + 1)}</ul>}
        </li>
      ));
  }

  return (
    <section className="card">
      <p>Choisis une action, puis place la fenêtre à examiner au premier plan dans les trois secondes.</p>
      <div className="section-header">
        <button disabled={busy || count > 0} onClick={() => start("inspect")}>
          Inspecter une fenêtre
        </button>
        <button disabled={busy || count > 0} onClick={() => start("capture")}>
          Capturer la fenêtre
        </button>
        <button type="button" disabled={busy || count > 0} onClick={() => void listDisplays()}>
          {displays ? "Actualiser les écrans" : "Lister les écrans"}
        </button>
        {count > 0 && <button onClick={() => setCount(0)}>Annuler</button>}
      </div>
      <div className="task-form">
        <label htmlFor="character-map-digit">Chiffre qui remplacera le champ « Caractères à copier »</label>
        <select
          id="character-map-digit"
          value={digit}
          disabled={busy || count > 0}
          onChange={(event) => setDigit(event.target.value)}
        >
          <option value="">Sélectionner un chiffre</option>
          {Array.from({ length: 10 }, (_, value) => (
            <option key={value}>{value}</option>
          ))}
        </select>
        <div>
          <button disabled={busy || count > 0 || digit === ""} onClick={() => start("input")}>
            Autoriser et saisir le chiffre
          </button>
        </div>
      </div>
      <p role="status">{count > 0 ? `${actionLabels[action]} dans ${count} s…` : message}</p>
      {displays && (
        <ul aria-label="Disposition des écrans">
          {displays.map((display, index) => {
            const { x, y, width, height, primary } = display;
            return (
              <li key={index}>
                <span>{primary ? "Écran principal" : `Écran ${index + 1}`}</span>
                <span>
                  {width} × {height} px · position ({x}, {y})
                </span>
                <button
                  type="button"
                  disabled={busy || count > 0}
                  aria-label={primary ? "Capturer l’écran principal entier" : `Capturer l’écran ${index + 1} entier`}
                  onClick={() => {
                    setSelectedDisplay(display);
                    start("display");
                  }}
                >
                  Capturer l’écran entier
                </button>
              </li>
            );
          })}
        </ul>
      )}
      {result && (
        <ul className="inspection-tree" aria-label="Structure de la fenêtre">
          {renderChildren(0)}
        </ul>
      )}
      {capture && (
        <CapturePreview
          capture={capture}
          fullDisplay={action === "display"}
          onClear={() => setCapture(undefined)}
          onFinished={onFinished}
        />
      )}
    </section>
  );
}

const visionErrors: Record<string, string> = {
  OLLAMA_CLOUD_NOT_DISABLED: "Démarre une version compatible d’Ollama avec OLLAMA_NO_CLOUD=1.",
  VISION_MODEL_REQUIRED: "Choisis un modèle de vision installé dans Ollama.",
  VISION_BUSY: "Une analyse est déjà en cours.",
};

function CapturePreview({
  capture,
  fullDisplay,
  onClear,
  onFinished,
}: {
  capture: Capture;
  fullDisplay: boolean;
  onClear: () => void;
  onFinished: () => void;
}) {
  const [url, setUrl] = useState("");
  const [models, setModels] = useState<string[]>([]);
  const [model, setModel] = useState("");
  const [message, setMessage] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    const objectUrl = URL.createObjectURL(new Blob([new Uint8Array(capture.jpeg)], { type: "image/jpeg" }));
    setUrl(objectUrl);
    return () => URL.revokeObjectURL(objectUrl);
  }, [capture]);

  async function discover() {
    setBusy(true);
    setDescription("");
    setMessage("Recherche des modèles de vision…");
    try {
      const names = await invoke<string[]>("discover_vision_models");
      setModels(names);
      setModel("");
      setMessage(names.length ? "" : "Installe un modèle de vision dans Ollama, puis relance la détection.");
    } catch (error) {
      setModels([]);
      setModel("");
      setMessage(visionErrors[String(error)] ?? "Impossible de détecter les modèles de vision Ollama.");
    } finally {
      setBusy(false);
    }
  }

  async function analyze(transcribe: boolean) {
    setBusy(true);
    setDescription("");
    setMessage(transcribe ? "Transcription de la capture…" : "Analyse de la capture…");
    try {
      setDescription(
        await invoke<string>(transcribe ? "transcribe_capture" : "analyze_capture", { model, jpeg: capture.jpeg }),
      );
      setMessage("");
    } catch (error) {
      setMessage(visionErrors[String(error)] ?? "Le modèle Ollama n’a pas pu analyser la capture.");
    } finally {
      setBusy(false);
      onFinished();
    }
  }

  return (
    <section aria-label="Aperçu local">
      <div className="section-header">
        <p>
          {capture.width} × {capture.height} px
        </p>
        <button onClick={onClear}>Effacer l’aperçu</button>
      </div>
      {url && (
        <img
          className="capture-preview"
          src={url}
          alt={fullDisplay ? "Capture de l’écran sélectionné" : "Capture de la fenêtre au premier plan"}
        />
      )}
      <div className="task-form">
        <label htmlFor="vision-model">Modèle de vision Ollama</label>
        <select
          id="vision-model"
          value={model}
          disabled={busy || !models.length}
          onChange={(event) => {
            setModel(event.target.value);
            setDescription("");
          }}
        >
          <option value="">Sélectionner un modèle</option>
          {models.map((name) => (
            <option key={name}>{name}</option>
          ))}
        </select>
        <div className="section-header">
          <button disabled={busy} onClick={() => void discover()}>
            Détecter les modèles
          </button>
          <button disabled={busy || !model} onClick={() => void analyze(false)}>
            Analyser avec Ollama
          </button>
          <button disabled={busy || !model} onClick={() => void analyze(true)}>
            Transcrire le texte
          </button>
        </div>
        <div role="status">{description ? <pre className="file-preview">{description}</pre> : message}</div>
      </div>
    </section>
  );
}
