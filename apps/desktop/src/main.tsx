import { Channel, invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { Inspector } from "./Inspector";
import { LocalFiles } from "./LocalFiles";
import { History } from "./History";
import { Security } from "./Security";
import "./style.css";

type Page = "Accueil" | "Modèles" | "Sécurité" | "Inspecteur" | "Fichiers" | "Activité";
type Provider = "ollama" | "openai";
type ModelPreferences = { provider: Provider; localModel: string; cloudModel: string; visionModel: string };
type Activity = { sequence: number; kind: string; timestamp: string; taskId: string | null; code: string | null };
type TaskEntry = { id: string; state: string; outcome: string | null; createdAt: string; updatedAt: string };
const taskStateNames: Record<string, string> = {
  created: "Créée",
  planning: "En cours",
  awaiting_approval: "En attente de ton accord",
  dispatching: "Action en cours",
  completed: "Terminée",
  failed: "En échec",
  cancelled: "Annulée",
  interrupted: "Interrompue (arrêt d’AiWin11)",
};
type Snapshot = { sessionId: string; events: Activity[] };
type ActionStatus = "done" | "partial" | "simulated" | "declined" | "expired" | "denied" | "failed";
type ActionReport = { action: string; summary: string; status: ActionStatus; code: string | null };
type TaskOutcome = { reply: string; actions: ActionReport[] };
const statusNames: Record<ActionStatus, string> = {
  done: "Fait",
  partial: "Non confirmé",
  simulated: "Simulé, rien n’a changé",
  declined: "Refusé par toi",
  expired: "Expiré sans réponse",
  denied: "Refusé par la politique",
  failed: "Échec",
};
type ApprovalAction =
  | { kind: "launch"; application: "notepad" }
  | { kind: "listFiles" }
  | { kind: "readFile"; name: string }
  | { kind: "copyFile"; source: string; destination: string; characters: number }
  | { kind: "moveFile"; source: string; destination: string; characters: number }
  | { kind: "trashFile"; name: string; characters: number }
  | { kind: "inspectWindow"; application: string; title: string }
  | { kind: "describeWindow"; application: string; title: string; model: string }
  | { kind: "typeText"; text: string; characters: number }
  | { kind: "listProcesses" }
  | { kind: "systemStatus" }
  | { kind: "readClipboard" }
  | { kind: "writeClipboard"; text: string; characters: number }
  | { kind: "listWindows" }
  | { kind: "listServices" }
  | { kind: "adminService"; service: string; operation: string }
  | { kind: "adminCleanTemp" }
  | { kind: "arrangeWindow"; application: string; title: string; arrangement: Arrangement };
type Arrangement = "focus" | "minimize" | "restore" | "maximize";
const arrangementNames: Record<Arrangement, string> = {
  focus: "mettre au premier plan",
  minimize: "réduire",
  restore: "restaurer",
  maximize: "agrandir",
};
type PendingApproval = { approvalId: string; action: ApprovalAction; expiresInMs: number };
type TaskUpdate =
  { type: "stage"; stage: string } | { type: "delta"; text: string } | ({ type: "approvalRequired" } & PendingApproval);
const applicationNames: Record<"notepad", string> = { notepad: "le Bloc-notes" };

function describeApproval(action: ApprovalAction): { title: string; detail: string; quote?: string } {
  if (action.kind === "typeText") {
    return {
      title: "L’agent propose d’écrire dans le Bloc-notes",
      detail:
        `Action : écrire ce texte (${action.characters} caractères) dans le Bloc-notes qu’il vient d’ouvrir. ` +
        "Rien n’est écrit si le document contient déjà du texte.",
      quote: action.text,
    };
  }
  if (action.kind === "launch") {
    const name = applicationNames[action.application];
    return {
      title: `L’agent propose d’ouvrir ${name}`,
      detail: `Action : lancer une nouvelle fenêtre de ${name}, sans argument.`,
    };
  }
  if (action.kind === "adminService" || action.kind === "adminCleanTemp") {
    return {
      title: "L’agent propose une opération administrateur",
      detail:
        (action.kind === "adminService"
          ? `Action : ${action.operation} sur le service « ${action.service} ».`
          : "Action : supprimer les fichiers temporaires de Windows de plus d’un jour (C:\\Windows\\Temp).") +
        " Si tu autorises, Windows te demandera aussi une confirmation (UAC). Une seule opération, puis l’assistant s’arrête.",
    };
  }
  if (action.kind === "listServices") {
    return {
      title: "L’agent demande à lister les services Windows",
      detail: "Action : lire les noms et l’état des services. Rien n’est démarré ni arrêté.",
    };
  }
  if (action.kind === "listWindows") {
    return {
      title: "L’agent demande à lister tes fenêtres",
      detail:
        "Action : lire le titre et l’application de chaque fenêtre ouverte (20 au plus). Les titres peuvent " +
        "contenir des noms de documents ou de sites ; ils restent sur ton PC (modèle local).",
    };
  }
  if (action.kind === "arrangeWindow") {
    return {
      title: `L’agent propose de ${arrangementNames[action.arrangement]} une fenêtre`,
      detail: `Fenêtre : « ${action.title} » (application ${action.application}). Elle n’est jamais fermée.`,
    };
  }
  if (action.kind === "readClipboard") {
    return {
      title: "L’agent demande à lire ton presse-papiers",
      detail:
        "Action : lire le texte actuellement copié (4000 caractères au plus) et le transmettre au modèle local. " +
        "Refuse s’il peut contenir un mot de passe ou une donnée privée.",
    };
  }
  if (action.kind === "writeClipboard") {
    return {
      title: "L’agent propose de copier un texte dans le presse-papiers",
      detail: `Action : remplacer le contenu du presse-papiers par ce texte (${action.characters} caractères). L’ancien contenu sera perdu.`,
      quote: action.text,
    };
  }
  if (action.kind === "systemStatus") {
    return {
      title: "L’agent demande à lire l’état du PC",
      detail: "Action : lire la mémoire, la charge du processeur et l’espace libre des disques. Rien n’est modifié.",
    };
  }
  if (action.kind === "listProcesses") {
    return {
      title: "L’agent demande à lister les processus",
      detail: "Action : lire les noms, la mémoire et le temps processeur des processus en cours. Rien n’est arrêté.",
    };
  }
  if (action.kind === "listFiles") {
    return {
      title: "L’agent demande à lister tes fichiers",
      detail: "Action : lister les fichiers .txt du dossier de Fichiers.",
    };
  }
  if (action.kind === "readFile") {
    return {
      title: "L’agent demande à lire un fichier",
      detail: `Action : lire « ${action.name} » dans le dossier de Fichiers.`,
    };
  }
  if (action.kind === "moveFile") {
    return {
      title: "L’agent propose de renommer un fichier",
      detail:
        `Action : renommer « ${action.source} » (${action.characters} caractères) en « ${action.destination} », ` +
        "dans le dossier de Fichiers. Aucun fichier existant ne sera remplacé.",
    };
  }
  if (action.kind === "trashFile") {
    return {
      title: "L’agent propose de mettre un fichier à la corbeille",
      detail:
        `Action : déplacer « ${action.name} » (${action.characters} caractères) dans la corbeille d’AiWin11. ` +
        "Le fichier n’est pas supprimé : il reste récupérable dans le dossier trash d’AiWin11.",
    };
  }
  if (action.kind === "describeWindow") {
    return {
      title: "L’agent demande à décrire une fenêtre",
      detail:
        `Fenêtre : « ${action.title} » (application ${action.application}), la dernière que tu utilisais ` +
        `avant AiWin11. Une capture de cette fenêtre sera analysée par le modèle de vision local ${action.model} ; ` +
        "l’agent ne recevra que la description. L’image n’est ni enregistrée ni envoyée ailleurs.",
    };
  }
  if (action.kind === "inspectWindow") {
    return {
      title: "L’agent demande à lire une fenêtre",
      detail:
        `Fenêtre : « ${action.title} » (application ${action.application}), la dernière que tu utilisais ` +
        "avant AiWin11. L’agent recevra les types et les noms de ses contrôles, sans capture d’écran ni " +
        "contenu des champs de mot de passe. Rien ne quitte ta machine.",
    };
  }
  return {
    title: "L’agent propose de copier un fichier",
    detail:
      `Action : copier « ${action.source} » (${action.characters} caractères) vers le nouveau fichier ` +
      `« ${action.destination} », dans le dossier de Fichiers. Aucun fichier existant ne sera remplacé.`,
  };
}
const MAX_PROMPT_CHARS = 2000;

const pages: Page[] = ["Accueil", "Modèles", "Sécurité", "Inspecteur", "Fichiers", "Activité"];
const activityNames: Record<string, string> = {
  "session.started": "Session ouverte",
  "task.created": "Demande reçue",
  "task.planning": "Consultation du modèle",
  "task.started": "Demande démarrée",
  "task.progress": "Étape de l’agent",
  "tasks.stop.requested": "Arrêt global demandé",
  "model.provider.unavailable": "Fournisseur de modèles indisponible",
  "settings.models.changed": "Choix des modèles modifié",
  "settings.retention.changed": "Durée de conservation modifiée",
  "history.purged": "Ancien historique supprimé (conservation)",
  "history.cleared": "Historique effacé",
  "history.exported": "Journal d’audit exporté",
  "process.list.completed": "Processus listés par l’agent",
  "process.list.failed": "Liste des processus impossible",
  "system.status.completed": "État du PC lu par l’agent",
  "system.status.failed": "État du PC indisponible",
  "clipboard.read.completed": "Presse-papiers lu par l’agent",
  "window.list.completed": "Fenêtres listées par l’agent",
  "service.list.completed": "Services listés par l’agent",
  "admin.operation.completed": "Opération administrateur effectuée",
  "admin.operation.failed": "Opération administrateur échouée ou refusée",
  "settings.admin.changed": "Opérations administrateur activées ou désactivées",
  "service.list.failed": "Liste des services impossible",
  "window.list.failed": "Liste des fenêtres impossible",
  "window.arrange.completed": "Fenêtre modifiée par l’agent",
  "window.arrange.partial": "Modification de fenêtre non vérifiée",
  "window.arrange.failed": "Modification de fenêtre impossible",
  "clipboard.read.failed": "Lecture du presse-papiers impossible",
  "clipboard.write.completed": "Texte copié dans le presse-papiers",
  "clipboard.write.partial": "Copie dans le presse-papiers non vérifiée",
  "clipboard.write.failed": "Copie dans le presse-papiers impossible",
  "grant.created": "Autorisation temporaire accordée",
  "grant.revoked": "Autorisation temporaire révoquée",
  "model.cloud.authorized": "Requête OpenAI autorisée",
  "model.cloud.denied": "Requête OpenAI refusée",
  "agent.tool.requested": "Action demandée par l’agent",
  "agent.reply": "Réponse de l’agent",
  "agent.observation": "Résultat transmis à l’agent",
  "window.locate.completed": "Fenêtre précédente identifiée",
  "window.locate.failed": "Aucune fenêtre précédente trouvée",
  "window.capture.completed": "Fenêtre capturée pour la vision locale",
  "window.capture.failed": "Capture de la fenêtre impossible",
  "vision.describe.completed": "Fenêtre décrite par le modèle de vision local",
  "vision.describe.failed": "Description de la fenêtre impossible",
  "window.inspect.completed": "Fenêtre lue par l’agent",
  "window.inspect.failed": "Lecture de la fenêtre impossible",
  "file.list.completed": "Fichiers listés par l’agent",
  "file.list.failed": "Liste des fichiers impossible",
  "policy.approval.required": "Approbation demandée",
  "policy.approval.granted": "Action approuvée par toi",
  "policy.approval.declined": "Action refusée par toi",
  "policy.approval.expired": "Approbation expirée",
  "settings.policy.changed": "Profil de sécurité modifié",
  "policy.evaluated": "Politique appliquée",
  "policy.simulated": "Action simulée, rien n’a été modifié",
  "policy.allowed": "Action autorisée",
  "policy.denied": "Action refusée",
  "tool.execution.completed": "Nouvelle fenêtre vérifiée",
  "tool.execution.partial": "Nouvelle fenêtre non confirmée",
  "tool.execution.failed": "Erreur du pont Windows",
  "task.completed": "Demande terminée",
  "task.failed": "Demande en échec",
  "task.partial": "Vérification incomplète",
  "task.cancelled": "Demande annulée",
  "task.recovered": "Demande interrompue retrouvée au démarrage",
  "policy.profile": "Profil de sécurité appliqué",
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
  "input.notepad.completed": "Texte écrit dans le Bloc-notes et vérifié",
  "input.notepad.partial": "Texte envoyé au Bloc-notes, non vérifié",
  "input.notepad.failed": "Écriture dans le Bloc-notes impossible",
  "file.move.completed": "Fichier renommé",
  "file.move.failed": "Échec du renommage",
  "file.trash.completed": "Fichier mis à la corbeille d’AiWin11",
  "file.trash.failed": "Échec de la mise à la corbeille",
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
  MODEL_UNAVAILABLE_OR_INVALID:
    "Ollama ou le modèle est indisponible, ou le modèle ne prend pas en charge les appels d’outils.",
  AGENT_UNAVAILABLE: "L’agent Python est indisponible. Vérifie son installation.",
  AGENT_REJECTED: "L’agent a rejeté la proposition du modèle : outil inconnu ou arguments invalides.",
  APPROVAL_NOT_PENDING: "Cette proposition n’est plus en attente.",
  TASK_BUSY: "Une demande est déjà en cours.",
  VISION_MODEL_REQUIRED: "Le modèle de vision choisi n’est pas valide.",
  AGENT_STEP_LIMIT: "L’agent n’a pas conclu dans le nombre d’étapes autorisé. Reformule ou précise ta demande.",
  FILE_UNAVAILABLE: "Le fichier ou le dossier de fichiers d’AiWin11 est indisponible.",
  FILE_DESTINATION_EXISTS: "Un fichier porte déjà ce nom. Rien n’a été remplacé.",
  FILE_COPY_FAILED: "La copie a échoué. Aucun fichier partiel n’a été conservé.",
  FILE_COPY_CLEANUP_FAILED: "La copie a échoué et un fichier temporaire n’a pas pu être supprimé.",
  FILE_OUTSIDE_SCOPE: "Le fichier se trouve hors du dossier autorisé ou n’est pas un fichier simple.",
  FILE_TOO_LARGE: "Le fichier dépasse 64 Kio.",
  FILE_NOT_UTF8: "Le fichier n’est pas un texte UTF-8.",
  INVALID_FILE_NAME: "Le nom de fichier n’est pas valide.",
  FILE_AUDIT_UNAVAILABLE:
    "Le journal local est indisponible ; l’opération n’a pas été effectuée ou n’a pas pu être journalisée.",
  TASK_CANCELLED: "Demande annulée. Aucune action n’a été exécutée.",
  BRIDGE_FAILED: "Le pont Windows n’a pas pu confirmer le résultat de l’action.",
  OUTCOME_UNRECORDED: "Le résultat de l’action n’a pas pu être journalisé.",
  AUDIT_UNAVAILABLE: "Le journal local est indisponible.",
};

function App() {
  const [page, setPage] = useState<Page>("Accueil");
  const [snapshot, setSnapshot] = useState<Snapshot>();
  const [tasks, setTasks] = useState<TaskEntry[]>([]);
  const [error, setError] = useState(false);
  const [models, setModels] = useState<string[]>([]);
  const [model, setModel] = useState("");
  const [visionModels, setVisionModels] = useState<string[]>([]);
  const [visionModel, setVisionModel] = useState("");
  const [visionStatus, setVisionStatus] = useState("");
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
  const [actionReports, setActionReports] = useState<ActionReport[]>([]);
  const [stopState, setStopState] = useState<"" | "idle" | "stopped" | "requested" | "confirmed" | "failed">("");
  const selectedModel = provider === "ollama" ? model : cloudModel.trim();
  const canSend =
    !running &&
    !!selectedModel &&
    !!prompt.trim() &&
    (provider === "ollama" || (cloudApproved && keyConfigured && !keyBusy));

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
    void invoke<TaskEntry[]>("get_tasks").then(setTasks, () => setTasks([]));
  }, []);

  useEffect(refresh, [refresh]);
  useEffect(() => {
    void refreshKey();
  }, []);
  useEffect(() => {
    // Restore the remembered choices, then check them against what is installed locally.
    void invoke<ModelPreferences>("get_model_preferences").then(
      (saved) => {
        setProvider(saved.provider === "openai" ? "openai" : "ollama");
        setModel(saved.localModel);
        setCloudModel(saved.cloudModel);
        setVisionModel(saved.visionModel);
        if (saved.provider !== "openai" && saved.localModel) void discoverModels();
        if (saved.provider !== "openai" && saved.visionModel) void discoverVisionModels();
      },
      () => undefined,
    );
  }, []);

  /** Remembers an explicit choice of the user (never the cloud approval nor the key). */
  function rememberModels(change: Partial<ModelPreferences>) {
    const next: ModelPreferences = { provider, localModel: model, cloudModel, visionModel, ...change };
    void invoke("set_model_preferences", { preferences: next }).catch(() => undefined);
  }
  async function discoverVisionModels() {
    setVisionStatus("Recherche en cours…");
    try {
      const available = await invoke<string[]>("discover_vision_models");
      setVisionModels(available);
      setVisionModel((current) => (available.includes(current) ? current : ""));
      setVisionStatus(available.length ? "" : "Aucun modèle de vision local installé.");
    } catch (failure) {
      setVisionModels([]);
      setVisionModel("");
      setVisionStatus(
        String(failure) === "OLLAMA_CLOUD_NOT_DISABLED"
          ? "Ollama doit tourner avec ses fonctions cloud désactivées (voir le README)."
          : "Les modèles de vision locaux sont indisponibles.",
      );
    }
  }

  async function discoverModels() {
    setLoadingModels(true);
    setCatalogStatus("Recherche en cours…");
    try {
      const available = await invoke<string[]>("discover_models");
      setModels(available);
      setModel((current) => (available.includes(current) ? current : ""));
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
    setActionReports([]);
    setStopState("");
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
      // Text streamed before a tool call is superseded by the next step.
      if (update.stage === "agent.tool.requested") setDraftReply("");
      setTaskProgress(activityNames[update.stage] ?? "");
      refresh();
    });
    try {
      const outcome = await invoke<TaskOutcome>("run_task", {
        request: {
          prompt,
          model: selectedModel,
          provider,
          cloudApproved,
          // Vision stays local: only sent for Ollama tasks.
          visionModel: provider === "ollama" && visionModel ? visionModel : null,
        },
        progress,
      });
      setAgentReply(outcome.reply);
      setActionReports(outcome.actions);
      setTaskStatus("");
      setPrompt("");
    } catch (failure) {
      setTaskStatus(taskErrors[String(failure)] ?? "Le cœur local n’a pas pu terminer la demande.");
    } finally {
      // The streamed draft is unvalidated: only the agent-approved reply stays on screen.
      setDraftReply("");
      setPendingApproval(undefined);
      setRunning(false);
      setStopState((current) => (current === "requested" ? "confirmed" : current));
      refresh();
    }
  }

  async function stopAll() {
    try {
      const status = await invoke<"idle" | "stopped" | "requested">("stop_all_tasks");
      setStopState(status);
    } catch {
      setStopState("failed");
    }
    refresh();
  }

  async function cancelTask() {
    try {
      const accepted = await invoke<boolean>("cancel_task");
      setTaskStatus((current) =>
        current === "Demande en cours…"
          ? accepted
            ? "Annulation demandée…"
            : "L’annulation n’est plus disponible pour cette demande."
          : current,
      );
    } catch {
      setTaskStatus((current) =>
        current === "Demande en cours…" ? "Le cœur local n’a pas pu traiter l’annulation." : current,
      );
    }
  }

  return (
    <div className="workspace">
      <aside>
        <div className="brand">
          <span aria-hidden="true">✦</span> AiWin11
        </div>
        <nav aria-label="Navigation principale">
          {pages.map((item) => (
            <button key={item} aria-current={item === page ? "page" : undefined} onClick={() => setPage(item)}>
              {item}
            </button>
          ))}
        </nav>
        <div className="connection">
          <span aria-hidden="true" />
          {provider === "ollama" ? "Ollama · Local" : "OpenAI · Cloud"}
        </div>
        <button className="stop-all" onClick={() => void stopAll()} aria-describedby="stop-status">
          Stoppe toutes les tâches
        </button>
        <p id="stop-status" role="status" className="stop-status">
          {
            {
              "": "",
              idle: "Aucune tâche en cours.",
              stopped: "Arrêt confirmé : la tâche est annulée.",
              requested: "Arrêt demandé : l’action en cours se termine, puis la tâche s’arrête.",
              confirmed: "Arrêt confirmé : aucune autre action n’a été lancée.",
              failed: "Le cœur local n’a pas pu traiter l’arrêt.",
            }[stopState]
          }
        </p>
      </aside>
      <main>
        <h1>{page === "Accueil" ? "Espace de travail" : page}</h1>
        {error && (
          <p role="alert">
            Le cœur local est indisponible. <button onClick={refresh}>Réessayer</button>
          </p>
        )}
        {page === "Accueil" && (
          <section className="card" aria-labelledby="session-heading">
            <div className="section-header">
              <h2 id="session-heading">Assistant</h2>
              <button className="link-button" onClick={() => setPage("Modèles")}>
                {selectedModel || "Choisir un modèle"}
              </button>
            </div>
            {lastPrompt && <p className="user-message">{lastPrompt}</p>}
            {agentReply && (
              <p className="agent-message" aria-label="Réponse de l’agent">
                {agentReply}
              </p>
            )}
            {actionReports.length > 0 && (
              <section className="action-report" aria-labelledby="report-heading">
                <h3 id="report-heading">Actions réellement effectuées</h3>
                <ul>
                  {actionReports.map((report, index) => (
                    <li key={index} className={`status-${report.status}`}>
                      <span>{report.summary}</span>
                      <strong>
                        {statusNames[report.status]}
                        {report.code && report.status !== "done" ? ` (${report.code})` : ""}
                      </strong>
                    </li>
                  ))}
                </ul>
              </section>
            )}
            {running && draftReply && (
              <p
                className="agent-message draft"
                aria-busy="true"
                aria-label="Réponse en cours de rédaction, non encore validée"
              >
                {draftReply}
              </p>
            )}
            {taskStatus && (
              <p role="status">
                {running && taskStatus === "Demande en cours…" && taskProgress ? taskProgress : taskStatus}
              </p>
            )}
            {pendingApproval && (
              <div
                className="approval-request"
                role="group"
                aria-labelledby="approval-heading"
                aria-describedby="approval-detail"
              >
                <h3 id="approval-heading">{describeApproval(pendingApproval.action).title}</h3>
                <p id="approval-detail">
                  {describeApproval(pendingApproval.action).detail} Ton accord vaut pour cette action uniquement. Sans
                  réponse, la proposition expire dans {Math.round(pendingApproval.expiresInMs / 1000)} secondes.
                </p>
                {describeApproval(pendingApproval.action).quote !== undefined && (
                  <blockquote className="approval-quote" aria-label="Texte qui sera écrit">
                    {describeApproval(pendingApproval.action).quote}
                  </blockquote>
                )}
                <div className="section-header">
                  <button type="button" disabled={approvalBusy} onClick={() => void resolveApproval(true)}>
                    Autoriser
                  </button>
                  {/* Focus starts on the safe choice, so Enter alone never approves. */}
                  <button type="button" autoFocus disabled={approvalBusy} onClick={() => void resolveApproval(false)}>
                    Refuser
                  </button>
                </div>
              </div>
            )}
            <form
              className="task-form"
              onSubmit={(event) => {
                event.preventDefault();
                if (canSend) void runTask();
              }}
            >
              <label htmlFor="prompt">Ta demande</label>
              <textarea
                id="prompt"
                rows={3}
                maxLength={MAX_PROMPT_CHARS}
                placeholder="Ouvre le Bloc-notes"
                value={prompt}
                disabled={running}
                onChange={(event) => {
                  setPrompt(event.target.value);
                  resetApproval();
                }}
              />
              {provider === "openai" && (
                <label className="approval">
                  <input
                    type="checkbox"
                    checked={cloudApproved}
                    disabled={running}
                    onChange={(event) => setCloudApproved(event.target.checked)}
                  />
                  Envoyer le texte de cette demande à OpenAI
                </label>
              )}
              <div className="section-header">
                <button type="submit" disabled={!canSend}>
                  Envoyer
                </button>
                {running && (
                  <button type="button" onClick={() => void cancelTask()}>
                    Annuler
                  </button>
                )}
              </div>
            </form>
          </section>
        )}
        {page === "Modèles" && (
          <section className="card">
            <div className="task-form">
              <label htmlFor="provider">Fournisseur pour les actions Windows</label>
              <select
                id="provider"
                value={provider}
                disabled={running || keyBusy}
                onChange={(event) => {
                  const chosen: Provider = event.target.value === "openai" ? "openai" : "ollama";
                  setProvider(chosen);
                  rememberModels({ provider: chosen });
                  resetApproval();
                }}
              >
                <option value="ollama">Ollama · Local</option>
                <option value="openai">OpenAI · Cloud</option>
              </select>
            </div>
            {provider === "ollama" ? (
              <>
                <div className="section-header">
                  <button onClick={() => void discoverModels()} disabled={loadingModels || running}>
                    Détecter les modèles
                  </button>
                </div>
                <p role="status">{catalogStatus}</p>
                <div className="task-form">
                  <label htmlFor="model">Modèle pour les actions Windows</label>
                  <select
                    id="model"
                    value={model}
                    disabled={!models.length || running || loadingModels}
                    onChange={(event) => {
                      setModel(event.target.value);
                      rememberModels({ localModel: event.target.value });
                      resetApproval();
                    }}
                  >
                    <option value="">Sélectionner un modèle</option>
                    {models.map((name) => (
                      <option key={name}>{name}</option>
                    ))}
                  </select>
                </div>
                <div className="section-header">
                  <button onClick={() => void discoverVisionModels()} disabled={running}>
                    Détecter les modèles de vision
                  </button>
                </div>
                <p role="status">{visionStatus}</p>
                <div className="task-form">
                  <label htmlFor="vision-model">Modèle de vision local (facultatif)</label>
                  <select
                    id="vision-model"
                    value={visionModel}
                    disabled={!visionModels.length || running}
                    onChange={(event) => {
                      setVisionModel(event.target.value);
                      rememberModels({ visionModel: event.target.value });
                    }}
                  >
                    <option value="">Aucun : l’agent ne peut pas décrire de fenêtre</option>
                    {visionModels.map((name) => (
                      <option key={name}>{name}</option>
                    ))}
                  </select>
                </div>
              </>
            ) : (
              <>
                <div className="task-form">
                  <label htmlFor="cloud-model">Identifiant du modèle OpenAI</label>
                  <input
                    id="cloud-model"
                    value={cloudModel}
                    maxLength={256}
                    disabled={running}
                    onChange={(event) => {
                      setCloudModel(event.target.value);
                      resetApproval();
                    }}
                    onBlur={() => rememberModels({ cloudModel: cloudModel.trim() })}
                  />
                </div>
                <form
                  className="task-form"
                  autoComplete="off"
                  onSubmit={(event) => {
                    event.preventDefault();
                    const field = event.currentTarget.elements.namedItem("api-key");
                    if (!(field instanceof HTMLInputElement) || !field.value.trim() || keyBusy || running) return;
                    const key = field.value;
                    field.value = "";
                    void updateKey(key);
                  }}
                >
                  <label htmlFor="api-key">Clé API OpenAI</label>
                  <input
                    id="api-key"
                    name="api-key"
                    type="password"
                    autoComplete="new-password"
                    maxLength={1024}
                    required
                    disabled={running || keyBusy}
                  />
                  <div className="section-header">
                    <button type="submit" disabled={running || keyBusy}>
                      Enregistrer la clé
                    </button>
                    {keyConfigured && (
                      <button type="button" disabled={running || keyBusy} onClick={() => void updateKey()}>
                        Supprimer la clé
                      </button>
                    )}
                  </div>
                  <p role="status">
                    {keyStatus || (keyConfigured ? "Clé enregistrée dans Windows." : "Aucune clé enregistrée.")}
                  </p>
                </form>
              </>
            )}
          </section>
        )}
        {page === "Sécurité" && <Security onFinished={refresh} />}
        {page === "Inspecteur" && <Inspector onFinished={refresh} />}
        {page === "Fichiers" && <LocalFiles onFinished={refresh} />}
        {page === "Activité" && (
          <>
            <section className="card" aria-labelledby="tasks-heading">
              <div className="section-header">
                <h2 id="tasks-heading">Demandes récentes</h2>
                <button onClick={refresh}>Actualiser</button>
              </div>
              {tasks.length ? (
                <ul>
                  {tasks.map((task) => (
                    <li key={task.id}>
                      <span>
                        {taskStateNames[task.state] ?? task.state}
                        {task.outcome && task.outcome !== "ok" ? ` (${task.outcome})` : ""}
                        {task.state === "interrupted" &&
                          task.outcome === "dispatching" &&
                          " — une action était peut-être en cours : vérifie son résultat"}
                      </span>
                      <time dateTime={task.createdAt}>{new Date(task.createdAt).toLocaleString("fr-FR")}</time>
                    </li>
                  ))}
                </ul>
              ) : (
                <p>Aucune demande enregistrée.</p>
              )}
            </section>
            <History running={running} onChange={refresh} />
            <section className="card" aria-labelledby="journal-heading">
              <h2 id="journal-heading">Journal de la session</h2>
              {snapshot?.events.length ? (
                <ul>
                  {snapshot.events.map((event) => (
                    <li key={event.sequence}>
                      <span>
                        {activityNames[event.kind] ?? event.kind}
                        {event.code && <code className="event-code">{event.code}</code>}
                      </span>
                      <time dateTime={event.timestamp}>{new Date(event.timestamp).toLocaleString("fr-FR")}</time>
                    </li>
                  ))}
                </ul>
              ) : (
                <p>Aucun événement à afficher.</p>
              )}
            </section>
          </>
        )}
      </main>
    </div>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("Racine de l’interface introuvable");
createRoot(root).render(<App />);
