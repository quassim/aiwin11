import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { agentState, agentStateLabels, transferLabel } from "./agentState";
import { BlockedApps } from "./BlockedApps";
import { ProtectedFiles, ProtectedWindows } from "./ProtectedFiles";
import { createRoot } from "react-dom/client";
import { Inspector } from "./Inspector";
import { LocalFiles } from "./LocalFiles";
import { Extensions } from "./Extensions";
import { History } from "./History";
import { Memory } from "./Memory";
import { Routines } from "./Routines";
import { ControlCenter } from "./ControlCenter";
import { Video } from "./Video";
import { ModelProbe } from "./ModelProbe";
import { ModelConfig, type ModelPreferences } from "./ModelConfig";
import { autoSpeak, speak, SpeechControls } from "./Speech";
import {
  Routing,
  type AgentModels,
  type CloudProvider,
  type LocalRuntimePreferences,
  type LocalStrategy,
} from "./Routing";
import { Security } from "./Security";
import "./style.css";

type Page =
  | "Accueil"
  | "Centre de contrôle"
  | "Modèles"
  | "Sécurité"
  | "Mémoire"
  | "Extensions"
  | "Inspecteur"
  | "Vidéo"
  | "Fichiers"
  | "Activité";
type Provider = "ollama" | "openai";
const workspacePages = {
  home: "Accueil",
  dashboard: "Centre de contrôle",
  models: "Modèles",
  security: "Sécurité",
  activity: "Activité",
} satisfies Record<string, Page>;
type WorkspaceRequest = { target: keyof typeof workspacePages; draft: string | null };
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
type Snapshot = { sessionId: string; events: Activity[]; replay: boolean };
type ActionStatus = "done" | "partial" | "simulated" | "declined" | "expired" | "denied" | "failed";
type ActionReport = { action: string; summary: string; status: ActionStatus; code: string | null };
type TaskOutcome = { reply: string; actions: ActionReport[] };
/** Why the local model was chosen for a request (journaled as model.selected). */
const routingReasons: Record<string, string> = {
  agent_profile: "Modèle configuré pour cet agent.",
  delegated_profile: "Modèle configuré pour l’agent délégué.",
  local_only: "Routage local uniquement.",
  cloud_only: "Routage cloud uniquement.",
  local_first: "Routage local d’abord.",
  balanced_cloud: "Modèle cloud vérifié sélectionné avec ton accord.",
  lm_studio_configured: "Modèle local configuré dans LM Studio.",
  gpu_verified: "Déjà chargé sur le GPU, appel d’outil vérifié.",
  quality_first: "Plus gros modèle local capable d’appeler des outils, appel d’outil vérifié.",
  cost_optimized: "Modèle déjà chargé en mémoire, le plus léger, appel d’outil vérifié.",
  configured_local: "Choix local configuré.",
  local_fallback: "Modèle local de secours : le modèle choisi a échoué, appel d’outil vérifié.",
};

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
  | { kind: "launch"; application: "notepad" | "charmap" | "paint" }
  | { kind: "listFiles" }
  | { kind: "readFile"; name: string }
  | { kind: "searchFiles"; query: string }
  | { kind: "copyFile"; source: string; destination: string; characters: number }
  | { kind: "moveFile"; source: string; destination: string; characters: number }
  | { kind: "trashFile"; name: string; characters: number }
  | { kind: "inspectWindow"; application: string; title: string }
  | { kind: "describeWindow"; application: string; title: string; model: string }
  | { kind: "typeText"; text: string; characters: number }
  | { kind: "listProcesses" }
  | { kind: "inspectProcess"; name: string; process_id: number }
  | { kind: "describeScreen"; model: string }
  | { kind: "listApplications" }
  | { kind: "stopProcess"; name: string; process_id: number }
  | { kind: "systemStatus" }
  | { kind: "readClipboard" }
  | { kind: "writeClipboard"; text: string; characters: number }
  | { kind: "listWindows" }
  | { kind: "listServices" }
  | { kind: "mcpListTools"; server: string }
  | { kind: "generateImage"; prompt: string }
  | { kind: "mcpCall"; server: string; tool: string; arguments: string }
  | { kind: "organizeFiles"; moves: [string, string][] }
  | { kind: "runDiagnostic"; diagnostic: string }
  | { kind: "remember"; text: string }
  | { kind: "recall" }
  | { kind: "searchMemory" }
  | { kind: "clickControl"; application: string; title: string; control: string }
  | { kind: "setControlValue"; application: string; title: string; control: string; text: string }
  | { kind: "adminService"; service: string; operation: string }
  | { kind: "adminCleanTemp" }
  | { kind: "arrangeWindow"; application: string; title: string; arrangement: Arrangement };
type Arrangement = "focus" | "minimize" | "restore" | "maximize" | "snap_left" | "snap_right" | "center";
const arrangementNames: Record<Arrangement, string> = {
  focus: "mettre au premier plan",
  minimize: "réduire",
  restore: "restaurer",
  maximize: "agrandir",
  snap_left: "placer sur la moitié gauche de l’écran",
  snap_right: "placer sur la moitié droite de l’écran",
  center: "centrer en taille moyenne",
};
type PendingApproval = { approvalId: string; action: ApprovalAction; expiresInMs: number };
type TaskUpdate =
  | { type: "stage"; stage: string }
  | { type: "delta"; text: string }
  | {
      type: "modelSelected";
      model: string;
      reason: string;
    }
  | ({ type: "approvalRequired"; issuedAtMs?: number } & PendingApproval);
type TaskProgress = { runId: string; sequence: number; update: TaskUpdate };
type TaskReplay = {
  runId: string;
  afterSequence: number;
  updates: TaskUpdate[];
  outcome: TaskOutcome | null;
  error: string | null;
};
const applicationNames: Record<"notepad" | "charmap" | "paint", string> = {
  notepad: "le Bloc-notes",
  charmap: "la Table des caractères",
  paint: "Paint",
};

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
  if (action.kind === "setControlValue") {
    return {
      title: "L’agent propose de remplir un champ",
      detail: `Remplacer le contenu de ${action.control} dans « ${action.title} » (${action.application}) par ce texte. Si UI Automation ne peut pas le faire, un champ Win32 vide peut recevoir une saisie ciblée.`,
      quote: action.text,
    };
  }
  if (action.kind === "clickControl") {
    return {
      title: "L’agent propose de cliquer dans une fenêtre",
      detail: `Cliquer sur ${action.control} dans « ${action.title} » (${action.application}). Si UI Automation ne peut pas le faire, un bouton Win32 peut recevoir un clic simulé ciblé.`,
    };
  }
  if (action.kind === "remember") {
    return {
      title: "L’agent propose de retenir une information",
      detail: "Action : enregistrer ce texte, chiffré, dans la mémoire d’AiWin11 (page Mémoire pour l’effacer).",
      quote: action.text,
    };
  }
  if (action.kind === "searchMemory") {
    return {
      title: "L’agent demande à chercher dans sa mémoire",
      detail: "Action : chercher parmi les informations retenues.",
    };
  }
  if (action.kind === "recall") {
    return { title: "L’agent demande à relire sa mémoire", detail: "Action : lire les informations retenues." };
  }
  if (action.kind === "runDiagnostic") {
    if (action.diagnostic === "powershell_os") {
      return {
        title: "Autoriser le diagnostic PowerShell",
        detail: "Lire l’édition, la version et le numéro de build de Windows.",
      };
    }
    return {
      title: "L’agent demande un diagnostic Windows",
      detail: `Action : lancer le diagnostic en lecture seule « ${action.diagnostic} ». Rien n’est modifié.`,
    };
  }
  if (action.kind === "organizeFiles") {
    return {
      title: `L’agent propose de ranger ${action.moves.length} fichier(s)`,
      detail: "Plan complet, approuvé en une fois. Aucun fichier existant ne sera remplacé :",
      quote: action.moves.map(([source, destination]) => `${source} → ${destination}`).join("\n"),
    };
  }
  if (action.kind === "generateImage") {
    return {
      title: "L’agent propose de générer une image",
      detail: "Action : générer une image avec ton moteur local et l’enregistrer dans le dossier d’images d’AiWin11.",
      quote: action.prompt,
    };
  }
  if (action.kind === "mcpListTools") {
    return {
      title: "L’agent demande à lister les outils d’une extension",
      detail: `Action : démarrer le serveur MCP « ${action.server} » pour lire la liste de ses outils.`,
    };
  }
  if (action.kind === "mcpCall") {
    return {
      title: "L’agent propose d’utiliser une extension",
      detail: `Action : appeler l’outil « ${action.tool} » du serveur MCP « ${action.server} » avec ces arguments. L’effet dépend entièrement de l’extension.`,
      quote: action.arguments,
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
  if (action.kind === "stopProcess") {
    return {
      title: `L’agent propose d’arrêter « ${action.name} »`,
      detail: `PID ${action.process_id}. S’il a une fenêtre, il reçoit seulement une demande de fermeture et peut te proposer d’enregistrer ; il n’est jamais forcé. Sans fenêtre, il est arrêté directement : un travail non enregistré serait perdu.`,
    };
  }
  if (action.kind === "listApplications") {
    return {
      title: "L’agent demande à lister les applications qu’il peut ouvrir",
      detail:
        "Action : vérifier si le Bloc-notes, la Table des caractères et Paint sont installés et ouverts. Rien n’est ouvert ni fermé.",
    };
  }
  if (action.kind === "describeScreen") {
    return {
      title: "L’agent demande à capturer tout l’écran principal",
      detail: `Image analysée localement par ${action.model}, puis effacée. Tout ce qui est visible à l’écran y figure ; refusé si une fenêtre protégée est visible.`,
    };
  }
  if (action.kind === "inspectProcess") {
    return {
      title: `L’agent demande à détailler le processus « ${action.name} »`,
      detail: `PID ${action.process_id}. Action : lire son exécutable, son éditeur, son parent, sa mémoire et ses threads. Rien n’est arrêté.`,
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
  if (action.kind === "searchFiles") {
    return {
      title: "L’agent demande à chercher dans tes fichiers",
      detail: `Action : chercher « ${action.query} » dans le nom et le contenu des fichiers du dossier de Fichiers.`,
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
const dictationErrors: Record<string, string> = {
  DICTATION_SILENCE: "Rien n’a été entendu.",
  DICTATION_NO_RECOGNIZER: "Aucun moteur de reconnaissance vocale Windows n’est installé pour cette langue.",
  DICTATION_NO_MICROPHONE: "Aucun micro disponible (ou accès au micro refusé dans les paramètres Windows).",
  CORE_BUSY: "AiWin11 est occupé : réessaie dans un instant.",
};

const pages: Page[] = [
  "Accueil",
  "Centre de contrôle",
  "Modèles",
  "Sécurité",
  "Mémoire",
  "Extensions",
  "Inspecteur",
  "Vidéo",
  "Fichiers",
  "Activité",
];
const expertPages = new Set<Page>(["Mémoire", "Extensions", "Inspecteur"]);
const activityNames: Record<string, string> = {
  "session.started": "Session ouverte",
  "task.created": "Demande reçue",
  "workflow.step.started": "Étape du workflow en cours",
  "workflow.step.completed": "Étape du workflow terminée",
  "workflow.step.failed": "Échec d’une étape du workflow",
  "workflow.step.skipped": "Étape conditionnelle ignorée",
  "task.planning": "Consultation du modèle",
  "task.started": "Demande démarrée",
  "task.progress": "Étape de l’agent",
  "tasks.stop.requested": "Arrêt global demandé",
  "model.provider.unavailable": "Fournisseur de modèles indisponible",
  "model.routing": "Sélection d’un modèle local",
  "model.selected": "Modèle local sélectionné",
  "settings.models.changed": "Choix des modèles modifié",
  "settings.privacy.changed": "Mode privé modifié",
  "agent.profile": "Agent choisi",
  "agent.delegated": "Sous-demande confiée à un agent spécialisé",
  "settings.routing.changed": "Routage des modèles modifié",
  "model.usage": "Jetons cloud utilisés",
  "model.budget.exceeded": "Budget cloud de la demande atteint",
  "settings.budget.changed": "Budget cloud modifié",
  "model.fallback.cloud": "Bascule vers le cloud (Ollama indisponible, autorisée)",
  "model.cloud.request": "Envoi de la demande au cloud",
  "model.fallback.local": "Bascule vers le modèle local de secours",
  "settings.retention.changed": "Durée de conservation modifiée",
  "history.purged": "Ancien historique supprimé (conservation)",
  "history.cleared": "Historique effacé",
  "history.exported": "Journal d’audit exporté",
  "process.stop.completed": "Processus arrêté",
  "process.stop.partial": "Fermeture demandée, processus toujours ouvert",
  "process.stop.failed": "Arrêt du processus impossible",
  "application.list.completed": "Applications listées",
  "application.list.failed": "Liste des applications impossible",
  "process.inspect.completed": "Processus détaillé",
  "process.inspect.failed": "Détail du processus impossible",
  "process.list.completed": "Processus listés par l’agent",
  "process.list.failed": "Liste des processus impossible",
  "system.status.completed": "État du PC lu par l’agent",
  "system.status.failed": "État du PC indisponible",
  "clipboard.read.completed": "Presse-papiers lu par l’agent",
  "window.list.completed": "Fenêtres listées par l’agent",
  "service.list.completed": "Services listés par l’agent",
  "mcp.request.completed": "Extension MCP utilisée",
  "image.generated": "Image générée localement",
  "image.failed": "Génération d’image impossible",
  "settings.image.changed": "Moteur d’images modifié",
  "mcp.request.failed": "Échec d’une extension MCP",
  "settings.mcp.changed": "Extensions MCP modifiées",
  "diagnostic.run.completed": "Diagnostic Windows effectué",
  "diagnostic.run.failed": "Diagnostic Windows impossible",
  "memory.saved": "Information retenue (chiffrée)",
  "memory.deleted": "Mémoire effacée",
  "settings.embedding.changed": "Recherche dans la mémoire modifiée",
  "control.find.completed": "Contrôles trouvés dans la fenêtre lue",
  "control.invoke.completed": "Action sur le contrôle effectuée",
  "control.invoke.partial": "Résultat de l’action sur le contrôle non vérifié",
  "control.invoke.failed": "Action sur le contrôle impossible",
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
  "model.cloud.authorized": "Requête cloud autorisée",
  "model.cloud.denied": "Requête cloud refusée",
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
  "file.search.completed": "Recherche dans les fichiers effectuée",
  "file.search.failed": "Recherche dans les fichiers impossible",
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
  "task.paused": "Demande en pause",
  "task.resumed": "Demande reprise",
  "task.recovering": "Nouvel essai après un échec passager",
  "task.recovery.finished": "Fin du nouvel essai",
  "task.recovered": "Demande interrompue retrouvée au démarrage",
  "policy.profile": "Profil de sécurité appliqué",
  "inspection.completed": "Fenêtre inspectée",
  "inspection.failed": "Inspection impossible",
  "capture.completed": "Fenêtre capturée",
  "capture.failed": "Capture impossible",
  "vision.completed": "Capture analysée localement",
  "vision.failed": "Analyse de la capture impossible",
  "ocr.completed": "Texte de la capture transcrit localement",
  "ocr.failed": "Transcription de la capture impossible",
  "displays.completed": "Disposition des écrans lue",
  "displays.failed": "Disposition des écrans indisponible",
  "display.capture.completed": "Écran capturé",
  "display.capture.failed": "Capture de l’écran impossible",
  "video.completed": "Vidéo générée localement",
  "video.failed": "Génération vidéo interrompue ou échouée",
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
  "file.trash.list.completed": "Corbeille consultée",
  "file.trash.list.failed": "Corbeille indisponible",
  "file.restore.completed": "Fichier restauré",
  "file.restore.failed": "Échec de la restauration",
};

const taskErrors: Record<string, string> = {
  PROMPT_INVALID: `Saisis une demande de ${MAX_PROMPT_CHARS} caractères au maximum.`,
  WORKFLOW_INVALID: "Vérifie que le workflow contient une à huit étapes numérotées et non vides.",
  WORKFLOW_LOCAL_ONLY: "Choisis un modèle Ollama pour exécuter ce workflow.",
  MODEL_REQUIRED: "Choisis un identifiant de modèle valide dans Modèles.",
  CLOUD_APPROVAL_REQUIRED: "Autorise la requête cloud pour cette demande.",
  CLOUD_KEY_REQUIRED: "Enregistre une clé API cloud dans Modèles.",
  CLOUD_PROVIDER_DISABLED: "Ce fournisseur cloud est désactivé dans Modèles.",
  ROUTING_LOCAL_REQUIRED: "La stratégie active exige un modèle Ollama.",
  ROUTING_CLOUD_REQUIRED: "La stratégie active exige un modèle cloud et ton accord.",
  CLOUD_BUDGET_EXCEEDED: "Le budget de jetons cloud de cette demande est atteint (page Modèles).",
  CLOUD_COST_EXCEEDED: "Le coût maximal fixé pour cette demande cloud est atteint (page Modèles).",
  AGENT_TIME_LIMIT: "La demande a dépassé 15 minutes : elle a été arrêtée.",
  PRIVATE_MODE: "Le mode privé est activé (page Sécurité) : aucune requête n’est envoyée au cloud.",
  CLOUD_REQUEST_FAILED: "La requête cloud a échoué. Vérifie la clé, l’accès au modèle et la connexion.",
  CLOUD_PROPOSAL_INVALID: "Le modèle cloud n’a renvoyé ni réponse complète ni appel d’outil valide.",
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
  FOLDER_UNKNOWN: "Ce dossier n’est pas un des dossiers de l’agent.",
  FILE_FOLDER_MISMATCH: "Une copie ou un renommage doit rester dans le même dossier.",
  FILE_PROTECTED: "Ce fichier est protégé par une règle de la page Sécurité : l’agent n’y a pas accès.",
  WINDOW_PROTECTED: "Cette fenêtre est protégée par une règle de la page Sécurité : l’agent n’y a pas accès.",
  FILE_TOO_LARGE: "Le fichier dépasse 64 Kio.",
  FILE_NOT_UTF8: "Le fichier n’est pas un texte UTF-8.",
  INVALID_FILE_NAME: "Le nom de fichier n’est pas valide.",
  FILE_AUDIT_UNAVAILABLE:
    "Le journal local est indisponible ; l’opération n’a pas été effectuée ou n’a pas pu être journalisée.",
  TASK_CANCELLED: "Demande annulée.",
  TASK_WORKER_UNAVAILABLE:
    "Le processus de tâche s’est arrêté. Consulte le journal pour vérifier les dernières actions.",
  PAUSE_EXPIRED: "La demande est restée en pause plus de 30 minutes : elle a été arrêtée.",
  BRIDGE_FAILED: "Le pont Windows n’a pas pu confirmer le résultat de l’action.",
  OUTCOME_UNRECORDED: "Le résultat de l’action n’a pas pu être journalisé.",
  AUDIT_UNAVAILABLE: "Le journal local est indisponible.",
};

function App() {
  const [page, setPage] = useState<Page>("Accueil");
  const pageHeading = useRef<HTMLHeadingElement>(null);
  const navigationStarted = useRef<number | undefined>(undefined);
  const navigate = (next: Page) => {
    if (next === page) return;
    navigationStarted.current = performance.now();
    setPage(next);
  };
  const [expert, setExpert] = useState(() => localStorage.getItem("aiwin11-expert") !== "false");
  const [snapshot, setSnapshot] = useState<Snapshot>();
  const auditSnapshot = useRef<Snapshot | undefined>(undefined);
  const auditRefresh = useRef(false);
  const auditRefreshQueued = useRef(false);
  const taskCursor = useRef({ runId: "", sequence: 0 });
  const localTaskInvocation = useRef(false);
  const taskReplayPending = useRef(false);
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
  const [cloudFallback, setCloudFallback] = useState(false);
  const [localStrategy, setLocalStrategy] = useState<LocalStrategy>();
  const [agentModels, setAgentModels] = useState<AgentModels>();
  const [localRuntime, setLocalRuntime] = useState<LocalRuntimePreferences>({ runtime: "ollama", lmStudioModel: "" });
  const localRuntimeName = localRuntime.runtime === "lmStudio" ? "LM Studio" : "Ollama";
  const taskProvider =
    localStrategy === "cloudOnly"
      ? "openai"
      : localStrategy === "localOnly" || localStrategy === "localFirst" || localStrategy === "balanced"
        ? "ollama"
        : provider;
  const [dictating, setDictating] = useState(false);
  const [floating, setFloating] = useState(false);
  const [agent, setAgent] = useState<"general" | keyof AgentModels>("general");
  const taskCloudFallback =
    localStrategy === "localFirst" ||
    (localStrategy === "manual" && cloudFallback) ||
    (localStrategy === "balanced" && agent === "general" && localRuntime.runtime === "ollama");
  const [dictationStatus, setDictationStatus] = useState("");
  const [vendor, setVendor] = useState("openai");
  const [cloudProviders, setCloudProviders] = useState<CloudProvider[]>([]);
  const vendorName = cloudProviders.find((entry) => entry.id === vendor)?.label ?? vendor;
  const vendorEnabled = cloudProviders.some((entry) => entry.id === vendor && entry.enabled);
  const [cloudProbeBusy, setCloudProbeBusy] = useState(false);
  const [cloudProbeStatus, setCloudProbeStatus] = useState("");
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
  const [routingExplanation, setRoutingExplanation] = useState("");
  const [agentReply, setAgentReply] = useState("");
  const [draftReply, setDraftReply] = useState("");
  const [actionReports, setActionReports] = useState<ActionReport[]>([]);
  const [stopState, setStopState] = useState<"" | "idle" | "stopped" | "requested" | "confirmed" | "failed">("");
  // "requested": pause asked, applied before the next model step; "paused": the task waits.
  const [pauseState, setPauseState] = useState<"" | "requested" | "paused">("");
  const [lastStage, setLastStage] = useState("");
  const [lastFailed, setLastFailed] = useState(false);
  const [sentToCloud, setSentToCloud] = useState(false);
  const [ranOnce, setRanOnce] = useState(false);
  const profileModel = agent === "general" ? "" : agentModels?.[agent];
  const selectedModel =
    taskProvider === "ollama"
      ? localRuntime.runtime === "lmStudio"
        ? localRuntime.lmStudioModel
        : profileModel || model
      : cloudModel.trim();
  const canSend =
    !running &&
    !!selectedModel &&
    !!prompt.trim() &&
    (taskProvider === "ollama" || (cloudApproved && keyConfigured && !keyBusy && vendorEnabled));

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
    if (auditRefresh.current) {
      auditRefreshQueued.current = true;
    } else {
      setError(false);
      auditRefresh.current = true;
      void (async () => {
        let page: Snapshot;
        do {
          auditRefreshQueued.current = false;
          const previous = auditSnapshot.current;
          page = await invoke<Snapshot>("get_snapshot", {
            sessionId: previous?.sessionId,
            afterSequence: previous?.events[0]?.sequence,
          });
          const combined =
            page.replay && previous?.sessionId === page.sessionId ? [...previous.events, ...page.events] : page.events;
          const unique = new Map(combined.map((event) => [event.sequence, event]));
          const latest = [...unique.values()].sort((left, right) => right.sequence - left.sequence).slice(0, 100);
          auditSnapshot.current = { ...page, events: latest };
          setSnapshot(auditSnapshot.current);
        } while (auditRefreshQueued.current || (page.replay && page.events.length === 100));
      })()
        .catch(() => setError(true))
        .finally(() => {
          auditRefresh.current = false;
        });
    }
    void invoke<TaskEntry[]>("get_tasks").then(setTasks, () => setTasks([]));
  }, []);

  useEffect(refresh, [refresh]);
  useEffect(() => {
    if (!error || page !== "Activité") return;
    const retry = window.setTimeout(refresh, 2000);
    return () => window.clearTimeout(retry);
  }, [error, page, refresh]);
  useEffect(() => {
    window.addEventListener("focus", refresh);
    window.addEventListener("online", refresh);
    return () => {
      window.removeEventListener("focus", refresh);
      window.removeEventListener("online", refresh);
    };
  }, [refresh]);
  useEffect(() => {
    let painted = 0;
    const frame = requestAnimationFrame(() => {
      painted = requestAnimationFrame(() => void invoke("ui_ready").catch(() => undefined));
    });
    return () => {
      cancelAnimationFrame(frame);
      cancelAnimationFrame(painted);
    };
  }, []);
  useEffect(() => {
    const start = navigationStarted.current;
    navigationStarted.current = undefined;
    if (start === undefined) return;
    pageHeading.current?.focus();
    let painted = 0;
    const frame = requestAnimationFrame(() => {
      painted = requestAnimationFrame(() => {
        void invoke("ui_navigation", { elapsedMs: Math.round(performance.now() - start) }).catch(() => undefined);
      });
    });
    return () => {
      cancelAnimationFrame(frame);
      cancelAnimationFrame(painted);
    };
  }, [page]);
  useEffect(() => {
    void invoke<LocalStrategy>("get_local_strategy").then(setLocalStrategy, () => undefined);
    void invoke<AgentModels>("get_agent_models").then(setAgentModels, () => undefined);
    void invoke<LocalRuntimePreferences>("get_local_runtime_preferences").then(setLocalRuntime, () => undefined);
  }, []);
  useEffect(() => {
    void invoke<string>("get_cloud_vendor").then(setVendor, () => undefined);
    void invoke<CloudProvider[]>("get_cloud_providers").then(setCloudProviders, () => undefined);
  }, []);
  useEffect(() => {
    // Ctrl+Alt+Espace from any application: open the request field, ready to type.
    let disposed = false;
    const acceptRequest = async () => {
      const request = await invoke<WorkspaceRequest | null>("take_workspace_request");
      if (disposed || !request) return;
      setPage(workspacePages[request.target]);
      if (request.draft !== null) {
        setPrompt(request.draft);
        setCloudApproved(false);
        requestAnimationFrame(() => document.getElementById("prompt")?.focus());
      }
    };
    const receive = () => void acceptRequest().catch(() => undefined);
    const subscription = listen("desktop-workspace-request", receive);
    void subscription
      .then(() => {
        if (!disposed) receive();
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      void subscription.then((unlisten) => unlisten()).catch(() => undefined);
    };
  }, []);
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

  const applyTaskUpdate = useCallback(
    ({ runId, sequence, update }: TaskProgress) => {
      const cursor = taskCursor.current;
      if (runId === cursor.runId && sequence <= cursor.sequence) return;
      // A later live fragment must not skip a missing one. Replay fetches the gap.
      if (sequence !== (runId === cursor.runId ? cursor.sequence + 1 : 1)) return;
      taskCursor.current = { runId, sequence };
      if (update.type === "modelSelected") {
        setRoutingExplanation(
          `Modèle utilisé : ${update.model}. ${routingReasons[update.reason] ?? "Choix local configuré."}`,
        );
        return;
      }
      if (update.type === "delta") {
        setDraftReply((current) => current + update.text);
        return;
      }
      if (update.type === "approvalRequired") {
        const { type: _type, issuedAtMs, ...approval } = update;
        const expiresInMs =
          issuedAtMs === undefined
            ? approval.expiresInMs
            : Math.max(0, approval.expiresInMs - (Date.now() - issuedAtMs));
        setDraftReply("");
        setPendingApproval(expiresInMs > 0 ? { ...approval, expiresInMs } : undefined);
        return;
      }
      setLastStage(update.stage);
      if (update.stage === "model.cloud.request") setSentToCloud(true);
      if (update.stage.startsWith("policy.approval.")) setPendingApproval(undefined);
      if (update.stage === "task.paused") setPauseState("paused");
      if (update.stage === "task.resumed") setPauseState("");
      // Text streamed before a tool call is superseded by the next step.
      if (update.stage === "agent.tool.requested" || update.stage === "workflow.step.started") setDraftReply("");
      setTaskProgress(activityNames[update.stage] ?? "");
      refresh();
    },
    [refresh],
  );

  const replayTask = useCallback(async () => {
    if (taskReplayPending.current) return;
    taskReplayPending.current = true;
    try {
      const cursor = taskCursor.current;
      const page = await invoke<TaskReplay>("get_task_replay", {
        runId: cursor.runId,
        afterSequence: cursor.sequence,
      });
      if (!page?.runId || (localTaskInvocation.current && page.runId !== taskCursor.current.runId)) return;
      for (const [index, update] of page.updates.entries()) {
        applyTaskUpdate({ runId: page.runId, sequence: page.afterSequence + index + 1, update });
      }
      if (localTaskInvocation.current) return;
      setRanOnce(true);
      const active = !page.outcome && !page.error;
      setRunning(active);
      if (active) {
        setTaskStatus("Demande en cours…");
      } else {
        setDraftReply("");
        setPendingApproval(undefined);
        setPauseState("");
        if (page.outcome) {
          setAgentReply(page.outcome.reply);
          setActionReports(page.outcome.actions);
          setTaskStatus("");
        } else if (page.error) {
          setTaskStatus(taskErrors[page.error] ?? "Le cœur local n’a pas pu terminer la demande.");
          setLastFailed(page.error !== "TASK_CANCELLED");
        }
      }
    } catch {
      // Keep the cursor. The next successful read resumes from the same sequence.
    } finally {
      taskReplayPending.current = false;
    }
  }, [applyTaskUpdate]);

  useEffect(() => {
    const reconnect = () => void replayTask();
    reconnect();
    window.addEventListener("focus", reconnect);
    window.addEventListener("online", reconnect);
    return () => {
      window.removeEventListener("focus", reconnect);
      window.removeEventListener("online", reconnect);
    };
  }, [replayTask]);

  useEffect(() => {
    if (!running) return;
    const timer = window.setInterval(() => void replayTask(), 600);
    return () => window.clearInterval(timer);
  }, [replayTask, running]);

  async function runTask() {
    localTaskInvocation.current = true;
    taskCursor.current = { runId: "", sequence: 0 };
    setRunning(true);
    resetApproval();
    setLastPrompt(prompt);
    setTaskStatus("Demande en cours…");
    setTaskProgress("");
    setRoutingExplanation("");
    setAgentReply("");
    setDraftReply("");
    setActionReports([]);
    setStopState("");
    setPauseState("");
    setLastStage("");
    setLastFailed(false);
    setSentToCloud(false);
    setRanOnce(true);
    setPendingApproval(undefined);
    const progress = new Channel<TaskProgress>(applyTaskUpdate);
    try {
      const outcome = await invoke<TaskOutcome>("run_task", {
        request: {
          prompt,
          model: selectedModel,
          provider: taskProvider,
          cloudApproved,
          cloudVendor: cloudApproved ? vendor : null,
          localRuntime: localRuntime.runtime,
          // Vision stays local: only sent for Ollama tasks.
          visionModel: taskProvider === "ollama" && visionModel ? visionModel : null,
          agent,
          // local_first: OpenAI only if Ollama is unreachable, with this request's approval.
          cloudFallbackModel:
            taskProvider === "ollama" && taskCloudFallback && cloudApproved && cloudModel.trim()
              ? cloudModel.trim()
              : null,
        },
        progress,
      });
      setAgentReply(outcome.reply);
      if (autoSpeak()) speak(outcome.reply);
      setActionReports(outcome.actions);
      setTaskStatus("");
      setPrompt("");
    } catch (failure) {
      setTaskStatus(taskErrors[String(failure)] ?? "Le cœur local n’a pas pu terminer la demande.");
      setLastFailed(String(failure) !== "TASK_CANCELLED");
    } finally {
      localTaskInvocation.current = false;
      // The streamed draft is unvalidated: only the agent-approved reply stays on screen.
      setDraftReply("");
      setPendingApproval(undefined);
      setPauseState("");
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

  async function togglePause() {
    try {
      if (pauseState === "") {
        if (await invoke<boolean>("pause_task")) setPauseState("requested");
      } else {
        await invoke<boolean>("resume_task");
        setPauseState("");
      }
    } catch {
      setTaskStatus("Le cœur local n’a pas pu traiter la pause.");
    }
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
      <button className="skip-link" type="button" onClick={() => pageHeading.current?.focus()}>
        Aller au contenu principal
      </button>
      <aside>
        <div className="brand">
          <span aria-hidden="true">✦</span> AiWin11
        </div>
        {(() => {
          const state = agentState({
            running,
            failed: lastFailed,
            paused: pauseState !== "",
            approvalPending: pendingApproval !== undefined,
            lastStage,
          });
          const transfer = transferLabel(running, sentToCloud, ranOnce);
          return (
            <div className={`agent-state agent-state-${state}`} role="status" aria-live="polite">
              <p>
                <span aria-hidden="true" className="agent-state-dot" /> Agent : {agentStateLabels[state]}
              </p>
              {transfer && <p className={sentToCloud ? "transfer transfer-cloud" : "transfer"}>{transfer}</p>}
            </div>
          );
        })()}
        <nav aria-label="Navigation principale">
          {pages
            .filter((item) => expert || !expertPages.has(item))
            .map((item) => (
              <button key={item} aria-current={item === page ? "page" : undefined} onClick={() => navigate(item)}>
                {item}
              </button>
            ))}
        </nav>
        <button
          type="button"
          className="link-button"
          aria-pressed={expert}
          onClick={() => {
            const next = !expert;
            setExpert(next);
            localStorage.setItem("aiwin11-expert", String(next));
            if (!next && expertPages.has(page)) navigate("Accueil");
          }}
        >
          Mode expert
        </button>
        <div className="connection">
          <span aria-hidden="true" />
          {taskProvider === "ollama" ? `${localRuntimeName} · Local` : `${vendorName} · Cloud`}
        </div>
        {running && <p role="status">{pendingApproval ? "En attente de ton accord" : "Assistant en cours"}</p>}
        <button className="link-button" onClick={() => void invoke("show_palette").catch(() => undefined)}>
          Palette de commandes
        </button>
        <button
          className="link-button"
          aria-pressed={floating}
          onClick={() => {
            const next = !floating;
            void invoke("set_floating", { enabled: next }).then(() => {
              setFloating(next);
              if (next) navigate("Accueil");
            });
          }}
        >
          {floating ? "Quitter le mode flottant" : "Mode flottant"}
        </button>
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
      <main aria-labelledby="page-heading">
        <h1 id="page-heading" tabIndex={-1} ref={pageHeading}>
          {page === "Accueil" ? "Espace de travail" : page}
        </h1>
        {error && (
          <p role="alert">
            Le cœur local est indisponible. <button onClick={refresh}>Réessayer</button>
          </p>
        )}
        {page === "Accueil" && (
          <section className="card" aria-labelledby="session-heading">
            <div className="section-header">
              <h2 id="session-heading">Assistant</h2>
              <button className="link-button" onClick={() => navigate("Modèles")}>
                {selectedModel || "Choisir un modèle"}
              </button>
            </div>
            {lastPrompt && <p className="user-message">{lastPrompt}</p>}
            {routingExplanation && <p role="status">{routingExplanation}</p>}
            {agentReply && (
              <p className="agent-message" aria-label="Réponse de l’agent">
                {agentReply}
              </p>
            )}
            {agentReply && <SpeechControls text={agentReply} />}
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
              <label htmlFor="agent">Agent</label>
              <select
                id="agent"
                value={agent}
                disabled={running}
                onChange={(event) => {
                  setAgent(event.target.value as typeof agent);
                  resetApproval();
                }}
              >
                <option value="general">Général : tous les outils</option>
                <option value="office">Fichiers et bureautique</option>
                <option value="windowsAdmin">Administration Windows</option>
                <option value="development">Développement : lecture des fichiers autorisés</option>
                <option value="research">Recherche : fichiers, mémoire et MCP</option>
                <option value="multimedia">Multimédia : images et inspection de fenêtres</option>
                <option value="readOnly">Lecture seule : aucune modification</option>
              </select>
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
              {taskProvider === "ollama" && taskCloudFallback && cloudModel.trim() && keyConfigured && (
                <label className="approval">
                  <input
                    type="checkbox"
                    checked={cloudApproved}
                    disabled={running}
                    onChange={(event) => setCloudApproved(event.target.checked)}
                  />
                  {localStrategy === "balanced"
                    ? "Autoriser le routage de cette demande vers"
                    : "Si le moteur local est indisponible, envoyer cette demande à"}{" "}
                  {vendorName} ({cloudModel.trim()})
                </label>
              )}
              {taskProvider === "openai" && (
                <label className="approval">
                  <input
                    type="checkbox"
                    checked={cloudApproved}
                    disabled={running}
                    onChange={(event) => setCloudApproved(event.target.checked)}
                  />
                  Envoyer le texte de cette demande à {vendorName}
                </label>
              )}
              <div className="section-header">
                <button type="submit" disabled={!canSend}>
                  Envoyer
                </button>
                <button
                  type="button"
                  disabled={running || dictating}
                  aria-label="Dicter la demande (micro, reconnaissance locale)"
                  onClick={() => {
                    setDictating(true);
                    setDictationStatus("Parle maintenant (10 secondes au plus)…");
                    void invoke<string>("dictate")
                      .then(
                        (text) => {
                          setPrompt((current) => (current.trim() ? `${current.trim()} ${text}` : text));
                          setDictationStatus("");
                        },
                        (failure) => setDictationStatus(dictationErrors[String(failure)] ?? "La dictée a échoué."),
                      )
                      .finally(() => setDictating(false));
                  }}
                >
                  {dictating ? "Écoute…" : "🎤 Dicter"}
                </button>
                {running && (
                  <button type="button" aria-pressed={pauseState !== ""} onClick={() => void togglePause()}>
                    {pauseState === "" ? "Pause" : "Reprendre"}
                  </button>
                )}
                {running && (
                  <button type="button" onClick={() => void cancelTask()}>
                    Annuler
                  </button>
                )}
              </div>
              {running && pauseState !== "" && (
                <p role="status">
                  {pauseState === "requested"
                    ? "Pause demandée : la demande s’arrêtera avant sa prochaine étape, sans interrompre une action en cours."
                    : "Demande en pause. Reprends-la ou annule-la ; au-delà de 30 minutes, elle s’arrête."}
                </p>
              )}
              <p role="status">{dictationStatus}</p>
            </form>
            <Routines
              prompt={prompt}
              onPick={(text) => {
                setPrompt(text);
                resetApproval();
                setTimeout(() => document.getElementById("prompt")?.focus(), 0);
              }}
            />
          </section>
        )}
        {page === "Centre de contrôle" && (
          <ControlCenter
            running={running}
            model={selectedModel}
            provider={taskProvider === "ollama" ? `${localRuntimeName} · Local` : `${vendorName} · Cloud`}
            agent={
              {
                general: "Général",
                office: "Fichiers et bureautique",
                windowsAdmin: "Administration Windows",
                development: "Développement",
                research: "Recherche",
                multimedia: "Multimédia",
                readOnly: "Lecture seule",
              }[agent]
            }
            status={pendingApproval ? "En attente de ton accord" : running ? "En cours" : "Disponible"}
            latestTask={tasks[0] ? (taskStateNames[tasks[0].state] ?? tasks[0].state) : undefined}
            onModels={() => navigate("Modèles")}
            onSecurity={() => navigate("Sécurité")}
            onActivity={() => navigate("Activité")}
          />
        )}
        {page === "Modèles" && (
          <Routing
            models={models}
            currentModel={model}
            onFallbackChange={setCloudFallback}
            strategy={localStrategy}
            agentModels={agentModels}
            onAgentModelsChange={setAgentModels}
            cloudProviders={cloudProviders}
            localRuntime={localRuntime}
            onLocalRuntimeChange={(preferences) => {
              setLocalRuntime(preferences);
              resetApproval();
            }}
            onCloudProviderChange={(id, enabled) => {
              setCloudProviders((entries) => entries.map((entry) => (entry.id === id ? { ...entry, enabled } : entry)));
              if (id === vendor) resetApproval();
            }}
            onStrategyChange={(strategy) => {
              setLocalStrategy(strategy);
              resetApproval();
            }}
          />
        )}
        {page === "Modèles" && (
          <ModelConfig
            preferences={{ provider, localModel: model, cloudModel, visionModel }}
            disabled={running || keyBusy}
            onImported={(imported) => {
              setProvider(imported.provider);
              setModel(imported.localModel);
              setVisionModel(imported.visionModel);
              setCloudModel(imported.cloudModel);
              resetApproval();
              if (imported.provider === "ollama") {
                void discoverModels();
                if (imported.visionModel) void discoverVisionModels();
              }
            }}
          />
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
                <option value="openai">{vendorName} · Cloud</option>
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
                <ModelProbe key={model} model={model} disabled={running || loadingModels} />
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
                  <label htmlFor="cloud-vendor">Fournisseur cloud</label>
                  <select
                    id="cloud-vendor"
                    value={vendor}
                    disabled={running || keyBusy}
                    onChange={(event) => {
                      const next = event.target.value;
                      void invoke("set_cloud_vendor", { vendor: next }).then(() => {
                        setVendor(next);
                        resetApproval();
                        void refreshKey();
                      });
                    }}
                  >
                    {cloudProviders.map((entry) => (
                      <option key={entry.id} value={entry.id} disabled={!entry.enabled}>
                        {entry.label}
                      </option>
                    ))}
                  </select>
                  <label htmlFor="cloud-model">Identifiant du modèle {vendorName}</label>
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
                  <button
                    type="button"
                    disabled={running || cloudProbeBusy || !keyConfigured || !vendorEnabled || !cloudModel.trim()}
                    onClick={() => {
                      setCloudProbeBusy(true);
                      setCloudProbeStatus("");
                      void invoke<boolean>("probe_cloud_model", { vendor, model: cloudModel.trim() })
                        .then(
                          (verified) =>
                            setCloudProbeStatus(
                              verified ? "Appel d’outil cloud vérifié." : "Appel d’outil attendu absent.",
                            ),
                          (failure) =>
                            setCloudProbeStatus(taskErrors[String(failure)] ?? "Le test du modèle a échoué."),
                        )
                        .finally(() => setCloudProbeBusy(false));
                    }}
                  >
                    Tester les appels d’outils cloud
                  </button>
                  <p>Envoie une demande synthétique à {vendorName} sans exécuter l’outil proposé.</p>
                  <p role="status">{cloudProbeStatus}</p>
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
                  <label htmlFor="api-key">Clé API {vendorName}</label>
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
        {page === "Sécurité" && (
          <>
            <Security onFinished={refresh} />
            <BlockedApps />
            <ProtectedFiles />
            <ProtectedWindows />
          </>
        )}
        {page === "Mémoire" && <Memory />}
        {page === "Extensions" && <Extensions />}
        {page === "Inspecteur" && <Inspector onFinished={refresh} />}
        {page === "Vidéo" && <Video onFinished={refresh} />}
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
