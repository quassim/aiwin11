import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type Job =
  | { status: "idle" | "submitting" | "queued" }
  | { status: "completed"; filename: string }
  | { status: "failed"; code: string; canResume: boolean };

const errors: Record<string, string> = {
  VIDEO_INVALID: "Vérifie le port et la description de la vidéo.",
  VIDEO_WORKFLOW_INVALID: "Le workflow API doit contenir un CLIPTextEncode avec le marqueur et un seul SaveVideo.",
  VIDEO_NODE_UNSUPPORTED: "Ce workflow contient un nœud hors de la liste des nœuds vidéo locaux pris en charge.",
  VIDEO_LIMIT_EXCEEDED:
    "Le workflow accepte une vidéo, jusqu’à 1 024 px par côté, 121 images et 50 étapes d’échantillonnage.",
  VIDEO_ENGINE_UNAVAILABLE: "ComfyUI ne répond pas sur le port local choisi.",
  VIDEO_WORKFLOW_REJECTED: "ComfyUI a refusé le workflow. Vérifie les modèles et les paramètres dans ComfyUI.",
  VIDEO_GENERATION_FAILED: "La génération a échoué dans ComfyUI.",
  VIDEO_OUTPUT_MISSING: "ComfyUI a terminé sans produire le MP4 attendu.",
  VIDEO_TIMEOUT: "Le suivi a expiré. Consulte la file de ComfyUI pour connaître l’état de la génération.",
  VIDEO_BUSY: "Une génération est déjà suivie.",
  VIDEO_TRACKING_PENDING: "Une vidéo soumise à ComfyUI attend la reprise du suivi.",
  VIDEO_QUOTA_EXCEEDED: "Quatre tentatives d’envoi ont déjà été réservées dans les dernières 24 heures.",
};

export function Video({ onFinished }: { onFinished: () => void }) {
  const [port, setPort] = useState("8188");
  const [workflow, setWorkflow] = useState("");
  const [prompt, setPrompt] = useState("");
  const [job, setJob] = useState<Job>({ status: "idle" });
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState("");
  const busy = submitting || job.status === "submitting" || job.status === "queued";
  const pending = job.status === "failed" && job.canResume;
  const portNumber = Number(port);
  const canSubmit =
    !busy &&
    !pending &&
    !!workflow.trim() &&
    !!prompt.trim() &&
    Number.isInteger(portNumber) &&
    portNumber >= 1 &&
    portNumber <= 65535;

  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    let wasRunning = false;
    async function poll() {
      try {
        const current = await invoke<Job>("video_status");
        if (!active) return;
        setJob(current);
        const running = current.status === "submitting" || current.status === "queued";
        if (wasRunning && !running) onFinished();
        wasRunning = running;
      } catch {
        // Keep the last known status until the native command answers again.
      } finally {
        if (active) timer = setTimeout(() => void poll(), 1500);
      }
    }
    void poll();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [onFinished]);

  async function start() {
    if (!canSubmit) return;
    setSubmitting(true);
    setError("");
    try {
      await invoke("start_video", { port: portNumber, workflow, prompt });
      setJob({ status: "submitting" });
    } catch (failure) {
      setError(errors[String(failure)] ?? "La génération n’a pas pu démarrer.");
    } finally {
      setSubmitting(false);
    }
  }

  async function tracking(action: "resume_video" | "forget_video_tracking") {
    setSubmitting(true);
    setError("");
    try {
      await invoke(action);
      setJob(await invoke<Job>("video_status"));
    } catch (failure) {
      setError(errors[String(failure)] ?? "Impossible de modifier le suivi.");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <section className="card">
      <h2>Génération locale avec ComfyUI</h2>
      <p>
        Dans ton workflow API, utilise <code>{"{{AIWIN11_PROMPT}}"}</code> comme texte du CLIPTextEncode à remplir.
      </p>
      <form
        className="task-form"
        onSubmit={(event) => {
          event.preventDefault();
          void start();
        }}
      >
        <label htmlFor="video-port">Port local de ComfyUI</label>
        <input
          id="video-port"
          type="number"
          min={1}
          max={65535}
          value={port}
          disabled={busy}
          onChange={(event) => setPort(event.target.value)}
        />
        <label htmlFor="video-workflow">Workflow ComfyUI au format API JSON</label>
        <textarea
          id="video-workflow"
          rows={6}
          maxLength={262144}
          value={workflow}
          disabled={busy}
          onChange={(event) => setWorkflow(event.target.value)}
        />
        <label htmlFor="video-prompt">Description de la vidéo</label>
        <textarea
          id="video-prompt"
          rows={3}
          maxLength={2000}
          value={prompt}
          disabled={busy}
          onChange={(event) => setPrompt(event.target.value)}
        />
        <button type="submit" disabled={!canSubmit}>
          Générer la vidéo
        </button>
      </form>
      <div role="status">
        {error ? (
          <p>{error}</p>
        ) : job.status === "submitting" ? (
          <p>Envoi vers ComfyUI…</p>
        ) : job.status === "queued" ? (
          <p>Dans la file ComfyUI. En attente du résultat…</p>
        ) : job.status === "completed" ? (
          <p>
            Vidéo disponible dans le dossier de sortie de ComfyUI <code>{job.filename}</code>.
          </p>
        ) : job.status === "failed" ? (
          <p>{errors[job.code] ?? "La génération n’a pas pu être confirmée."}</p>
        ) : null}
      </div>
      {pending && (
        <div className="section-header">
          <button disabled={busy} onClick={() => void tracking("resume_video")}>
            Reprendre le suivi
          </button>
          <button
            disabled={busy}
            onClick={() => {
              if (window.confirm("Oublier le suivi ? Le travail peut encore continuer dans ComfyUI.")) {
                void tracking("forget_video_tracking");
              }
            }}
          >
            Oublier le suivi
          </button>
        </div>
      )}
    </section>
  );
}
