import { Channel, invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { Inspector } from "./Inspector";
import { LocalFiles } from "./LocalFiles";
import "./style.css";

type Page = "Accueil" | "Modèles" | "Inspecteur" | "Fichiers" | "Activité";
type Provider = "ollama" | "openai";
type Activity = { sequence: number; kind: string; timestamp: string };
type Snapshot = { sessionId: string; events: Activity[] };
type TaskOutcome =
  | { kind: "launched"; application: "notepad"; processId: number }
  | { kind: "reply"; text: string };
type PendingApproval = { approvalId: string; tool: "launch_application"; application: "notepad"; expiresInMs: number };
type TaskUpdate =
  | { type: "stage"; stage: string }
  | { type: "delta"; text: string }
  | ({ type: "approvalRequired" } & PendingApproval);
const applicationNames: Record<PendingApproval["application"], string> = { notepad: "le Bloc-notes" };
const MAX_PROMPT_CHARS = 2000;

const pages: Page[] = ["Accueil", "Modèles", "Inspecteur", "Fichiers", "Activité"];
const activityNames: Record<string, string> = {
  "session.started": "Session ouverte",
  "task.created": "Demande reçue",
  "task.planning": "Consultation du modèle",
  "model.cloud.authorized": "Requête OpenAI autorisée",
  "model.cloud.denied": "Requête OpenAI refusée",
  "agent.tool.requested": "Action demandée par l’agent",
  "agent.reply": "Réponse de l’agent",
  "policy.approval.required": "Approbation demandée",
  "policy.approval.granted": "Action approuvée par toi",
  "policy.approval.declined": "Action refusée par toi",
  "policy.approval.expired": "Approbation expirée",
  "policy.allowed": "Action autorisée",
  "policy.denied": "Action refusée",
  "tool.execution.completed": "Nouvelle fenêtre vérifiée",
  "tool.execution.partial": "Nouvelle fenêtre non confirmée",
  "tool.execution.failed": "Erreur du pont Windows",
  "task.completed": "Demande terminée",
  "task.failed": "Demande en échec",
  "task.partial": "Vérification incomplète",
  "task.cancelled": "Demande annulée",
  "inspection.completed": "Fenêtre inspectée",
  "inspection.failed": "Inspection impossible",
  "capture.completed": "Fenêtre capturée",
  "capture.failed": "Capture impossible",
  "vision.completed": "Capture analysée localement",
  "vision.failed": "Analyse de la capture impossible",
  "input.completed": "Saisie dans la Table des caractères vérifiée",
  "input.partial": "Saisie envoyée, champ non confirmé",
  "input.failed": "Saisie dans la Table des caractères non confirmée",
  "file.preview.completed": "Aperçu de fichier lu",
  "file.preview.failed": "Aperçu de fichier indisponible",
  "file.copy.completed": "Fichier copié",
  "file.copy.failed": "Échec de la copie",
};

const taskErrors: Record<string, string> = {
  PROMPT_INVALID: `Saisis une demande de ${MAX_PROMPT_CHARS} caractères au maximum.`,
  MODEL_REQUIRED: "Choisis un identifiant de modèle valide dans Modèles.",
  CLOUD_APPROVAL_REQUIRED: "Autorise la requête OpenAI pour cette demande.",
  CLOUD_KEY_REQUIRED: "Enregistre une clé API OpenAI dans Modèles.",
  CLOUD_REQUEST_FAILED: "La requête OpenAI a échoué. Vérifie la clé, l’accès au modèle et la connexion.",
  CLOUD_PROPOSAL_INVALID: "Le modèle OpenAI n’a renvoyé ni réponse complète ni appel d’outil valide.",
  CREDENTIAL_STORE_UNAVAILABLE: "Le Gestionnaire d’identifiants Windows est indisponible.",
  INVALID_API_KEY: "Le format de la clé API est invalide.",
  MODEL_UNAVAILABLE_OR_INVALID: "Ollama ou le modèle est indisponible, ou le modèle ne prend pas en charge les appels d’outils.",
  AGENT_UNAVAILABLE: "L’agent Python est indisponible. Vérifie son installation.",
  AGENT_REJECTED: "L’agent a rejeté la proposition du modèle : outil inconnu ou arguments invalides.",
  ACTION_DENIED: "Le broker a refusé l’action proposée.",
  ACTION_DECLINED: "Action refusée. Rien n’a été exécuté.",
  APPROVAL_EXPIRED: "La proposition a expiré sans réponse. Rien n’a été exécuté.",
  APPROVAL_NOT_PENDING: "Cette proposition n’est plus en attente.",
  TASK_BUSY: "Une demande est déjà en cours.",
  TASK_CANCELLED: "Demande annulée. Aucune action n’a été exécutée.",
  WINDOW_UNVERIFIED: "Windows a reçu la demande. Aucune nouvelle fenêtre du Bloc-notes n’a pu être confirmée.",
  BRIDGE_FAILED: "Le pont Windows n’a pas pu confirmer le résultat de l’action.",
  OUTCOME_UNRECORDED: "Le résultat de l’action n’a pas pu être journalisé.",
  AUDIT_UNAVAILABLE: "Le journal local est indisponible.",
};

function App() {
  const [page, setPage] = useState<Page>("Accueil");
  const [snapshot, setSnapshot] = useState<Snapshot>();
  const [error, setError] = useState(false);
  const [models, setModels] = useState<string[]>([]);
  const [model, setModel] = useState("");
  const [provider, setProvider] = useState<Provider>("ollama");
  const [cloudModel, setCloudModel] = useState("");
  const [cloudApproved, setCloudApproved] = useState(false);
  const [keyConfigured, setKeyConfigured] = useState(false);
  const [keyBusy, setKeyBusy] = useState(false);
  const [keyStatus, setKeyStatus] = useState("");
  const [catalogStatus, setCatalogStatus] = useState("");
  const [loadingModels, setLoadingModels] = useState(false);
  const [prompt, setPrompt] = useState("");
  const [pendingApproval, setPendingApproval] = useState<PendingApproval>();
  const [approvalBusy, setApprovalBusy] = useState(false);
  const [running, setRunning] = useState(false);
  const [lastPrompt, setLastPrompt] = useState("");
  const [taskStatus, setTaskStatus] = useState("");
  const [taskProgress, setTaskProgress] = useState("");
  const [agentReply, setAgentReply] = useState("");
  const [draftReply, setDraftReply] = useState("");
  const selectedModel = provider === "ollama" ? model : cloudModel.trim();
  const canSend = !running && !!selectedModel && !!prompt.trim()
    && (provider === "ollama" || (cloudApproved && keyConfigured && !keyBusy));

  function resetApproval() {
    setCloudApproved(false);
  }

  async function resolveApproval(approved: boolean) {
    if (!pendingApproval || approvalBusy) return;
    setApprovalBusy(true);
    try {
      await invoke("resolve_approval", { approvalId: pendingApproval.approvalId, approved });
      setPendingApproval(undefined);
    } catch (failure) {
      setPendingApproval(undefined);
      setTaskStatus(taskErrors[String(failure)] ?? "La décision n’a pas pu être transmise.");
    } finally {
      setApprovalBusy(false);
    }
  }

  async function refreshKey() {
    try {
      setKeyConfigured(await invoke<boolean>("openai_key_configured"));
    } catch {
      setKeyConfigured(false);
      setKeyStatus(taskErrors.CREDENTIAL_STORE_UNAVAILABLE);
    }
  }

  async function updateKey(key?: string) {
    setKeyBusy(true);
    resetApproval();
    try {
      if (key === undefined) await invoke("delete_openai_key");
      else await invoke("save_openai_key", { key });
      setKeyConfigured(key !== undefined);
      setKeyStatus("");
    } catch (failure) {
      setKeyStatus(taskErrors[String(failure)] ?? "Impossible de modifier la clé API.");
    } finally {
      setKeyBusy(false);
    }
  }

  const refresh = useCallback(() => {
    setError(false);
    void invoke<Snapshot>("get_snapshot").then(setSnapshot, () => setError(true));
  }, []);

  useEffect(refresh, [refresh]);
  useEffect(() => { void refreshKey(); }, []);
  async function discoverModels() {
    setLoadingModels(true);
    setCatalogStatus("Recherche en cours…");
    try {
      const available = await invoke<string[]>("discover_models");
      setModels(available);
      setModel((current) => available.includes(current) ? current : "");
      setCatalogStatus(available.length ? "" : "Aucun modèle local installé.");
    } catch {
      setModels([]);
      setModel("");
      setCatalogStatus("Ollama est indisponible sur cette machine.");
    } finally {
      setLoadingModels(false);
    }
  }

  async function runTask() {
    setRunning(true);
    resetApproval();
    setLastPrompt(prompt);
    setTaskStatus("Demande en cours…");
    setTaskProgress("");
    setAgentReply("");
    setDraftReply("");
    setPendingApproval(undefined);
    const progress = new Channel<TaskUpdate>((update) => {
      if (update.type === "delta") {
        setDraftReply((current) => current + update.text);
        return;
      }
      if (update.type === "approvalRequired") {
        const { type: _type, ...approval } = update;
        setDraftReply("");
        setPendingApproval(approval);
        return;
      }
      if (update.stage.startsWith("policy.approval.")) setPendingApproval(undefined);
      setTaskProgress(activityNames[update.stage] ?? "");
      refresh();
    });
    try {
      const outcome = await invoke<TaskOutcome>("run_task", {
        request: { prompt, model: selectedModel, provider, cloudApproved },
        progress,
      });
      if (outcome.kind === "reply") {
        setAgentReply(outcome.text);
        setTaskStatus("");
      } else {
        setTaskStatus("Le Bloc-notes est ouvert. Une nouvelle fenêtre a été vérifiée.");
      }
      setPrompt("");
    } catch (failure) {
      setTaskStatus(taskErrors[String(failure)] ?? "Le cœur local n’a pas pu terminer la demande.");
    } finally {
      // The streamed draft is unvalidated: only the agent-approved reply stays on screen.
      setDraftReply("");
      setPendingApproval(undefined);
      setRunning(false);
      refresh();
    }
  }

  async function cancelTask() {
    try {
      const accepted = await invoke<boolean>("cancel_task");
      setTaskStatus((current) => current === "Demande en cours…" ? (
        accepted ? "Annulation demandée…" : "L’annulation n’est plus disponible pour cette demande."
      ) : current);
    } catch {
      setTaskStatus((current) => current === "Demande en cours…" ? "Le cœur local n’a pas pu traiter l’annulation." : current);
    }
  }

  return (
    <div className="workspace">
      <aside>
        <div className="brand"><span aria-hidden="true">✦</span> AiWin11</div>
        <nav aria-label="Navigation principale">
          {pages.map((item) => (
            <button key={item} aria-current={item === page ? "page" : undefined}
              onClick={() => setPage(item)}>{item}</button>
          ))}
        </nav>
        <div className="connection"><span aria-hidden="true" />{provider === "ollama" ? "Ollama · Local" : "OpenAI · Cloud"}</div>
      </aside>
      <main>
        <h1>{page === "Accueil" ? "Espace de travail" : page}</h1>
        {error && <p role="alert">Le cœur local est indisponible. <button onClick={refresh}>Réessayer</button></p>}
        {page === "Accueil" && (
          <section className="card" aria-labelledby="session-heading">
            <div className="section-header">
              <h2 id="session-heading">Assistant</h2>
              <button className="link-button" onClick={() => setPage("Modèles")}>{selectedModel || "Choisir un modèle"}</button>
            </div>
            {lastPrompt && <p className="user-message">{lastPrompt}</p>}
            {agentReply && <p className="agent-message" aria-label="Réponse de l’agent">{agentReply}</p>}
            {running && draftReply && <p className="agent-message draft" aria-busy="true"
              aria-label="Réponse en cours de rédaction, non encore validée">{draftReply}</p>}
            {taskStatus && <p role="status">
              {running && taskStatus === "Demande en cours…" && taskProgress ? taskProgress : taskStatus}
            </p>}
            {pendingApproval && (
              <div className="approval-request" role="group" aria-labelledby="approval-heading"
                aria-describedby="approval-detail">
                <h3 id="approval-heading">L’agent propose d’ouvrir {applicationNames[pendingApproval.application]}</h3>
                <p id="approval-detail">
                  Action : lancer une nouvelle fenêtre de {applicationNames[pendingApproval.application]}, sans argument.
                  Ton accord vaut pour cette action uniquement. Sans réponse, la proposition expire
                  dans {Math.round(pendingApproval.expiresInMs / 1000)} secondes.
                </p>
                <div className="section-header">
                  <button type="button" disabled={approvalBusy}
                    onClick={() => void resolveApproval(true)}>Autoriser</button>
                  {/* Focus starts on the safe choice, so Enter alone never approves. */}
                  <button type="button" autoFocus disabled={approvalBusy}
                    onClick={() => void resolveApproval(false)}>Refuser</button>
                </div>
              </div>
            )}
            <form className="task-form" onSubmit={(event) => {
              event.preventDefault();
              if (canSend) void runTask();
            }}>
              <label htmlFor="prompt">Ta demande</label>
              <textarea id="prompt" rows={3} maxLength={MAX_PROMPT_CHARS} placeholder="Ouvre le Bloc-notes"
                value={prompt} disabled={running} onChange={(event) => { setPrompt(event.target.value); resetApproval(); }} />
              {provider === "openai" && <label className="approval">
                <input type="checkbox" checked={cloudApproved} disabled={running}
                  onChange={(event) => setCloudApproved(event.target.checked)} />
                Envoyer le texte de cette demande à OpenAI
              </label>}
              <div className="section-header">
                <button type="submit" disabled={!canSend}>Envoyer</button>
                {running && <button type="button" onClick={() => void cancelTask()}>Annuler</button>}
              </div>
            </form>
          </section>
        )}
        {page === "Modèles" && (
          <section className="card">
            <div className="task-form">
              <label htmlFor="provider">Fournisseur pour les actions Windows</label>
              <select id="provider" value={provider} disabled={running || keyBusy} onChange={(event) => {
                setProvider(event.target.value === "openai" ? "openai" : "ollama");
                resetApproval();
              }}>
                <option value="ollama">Ollama · Local</option>
                <option value="openai">OpenAI · Cloud</option>
              </select>
            </div>
            {provider === "ollama" ? <>
            <div className="section-header">
              <button onClick={() => void discoverModels()} disabled={loadingModels || running}>Détecter les modèles</button>
            </div>
            <p role="status">{catalogStatus}</p>
            <div className="task-form">
              <label htmlFor="model">Modèle pour les actions Windows</label>
              <select id="model" value={model} disabled={!models.length || running || loadingModels}
                onChange={(event) => { setModel(event.target.value); resetApproval(); }}>
                <option value="">Sélectionner un modèle</option>
                {models.map((name) => <option key={name}>{name}</option>)}
              </select>
            </div>
            </> : <>
              <div className="task-form">
                <label htmlFor="cloud-model">Identifiant du modèle OpenAI</label>
                <input id="cloud-model" value={cloudModel} maxLength={256} disabled={running}
                  onChange={(event) => { setCloudModel(event.target.value); resetApproval(); }} />
              </div>
              <form className="task-form" autoComplete="off" onSubmit={(event) => {
                event.preventDefault();
                const field = event.currentTarget.elements.namedItem("api-key");
                if (!(field instanceof HTMLInputElement) || !field.value.trim() || keyBusy || running) return;
                const key = field.value;
                field.value = "";
                void updateKey(key);
              }}>
                <label htmlFor="api-key">Clé API OpenAI</label>
                <input id="api-key" name="api-key" type="password" autoComplete="new-password"
                  maxLength={1024} required disabled={running || keyBusy} />
                <div className="section-header">
                  <button type="submit" disabled={running || keyBusy}>Enregistrer la clé</button>
                  {keyConfigured && <button type="button" disabled={running || keyBusy}
                    onClick={() => void updateKey()}>Supprimer la clé</button>}
                </div>
                <p role="status">{keyStatus || (keyConfigured ? "Clé enregistrée dans Windows." : "Aucune clé enregistrée.")}</p>
              </form>
            </>}
          </section>
        )}
        {page === "Inspecteur" && <Inspector onFinished={refresh} />}
        {page === "Fichiers" && <LocalFiles onFinished={refresh} />}
        {page === "Activité" && (
          <section className="card">
            <div className="section-header"><h2>Journal de la session</h2><button onClick={refresh}>Actualiser</button></div>
            {snapshot?.events.length ? (
              <ul>{snapshot.events.map((event) => (
                <li key={event.sequence}>
                  <span>{activityNames[event.kind] ?? event.kind}</span>
                  <time dateTime={event.timestamp}>{new Date(event.timestamp).toLocaleString("fr-FR")}</time>
                </li>
              ))}</ul>
            ) : <p>Aucun événement à afficher.</p>}
          </section>
        )}
      </main>
    </div>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("Racine de l’interface introuvable");
createRoot(root).render(<App />);
