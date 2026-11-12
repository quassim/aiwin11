import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { AdminToggle, PrivacyToggle } from "./AdminToggle";
import { Grants } from "./Grants";

type Autonomy = "observation" | "assisted" | "limited" | "advanced";
type Effect = "allow" | "ask" | "deny" | "simulate";
type PolicySettings = { autonomy: Autonomy; simulate: boolean };
type PolicyRow = { action: string; effects: [Effect, Effect, Effect, Effect] };
type Risk = "low" | "medium" | "high" | "critical";
type Reversibility = "noEffect" | "reversible" | "recoverable" | "irreversible";
type ToolManifest = {
  action: string;
  tool: string;
  version: number;
  risk: Risk;
  sideEffect: boolean;
  readsScreen: boolean;
  localOnly: boolean;
  dryRun: boolean;
  reversibility: Reversibility;
  timeoutMs: number;
};
type PolicyOverview = { settings: PolicySettings; table: PolicyRow[]; manifests: ToolManifest[] };

const riskNames: Record<Risk, string> = { low: "Faible", medium: "Moyen", high: "Élevé", critical: "Critique" };
const reversibilityNames: Record<Reversibility, string> = {
  noEffect: "Sans effet (lecture)",
  reversible: "Réversible",
  recoverable: "Récupérable (corbeille)",
  irreversible: "Irréversible",
};

const profiles: { id: Autonomy; label: string; description: string }[] = [
  {
    id: "observation",
    label: "Observation",
    description: "L’agent lit, mais ne modifie rien : toute action avec effet est refusée.",
  },
  {
    id: "assisted",
    label: "Assisté",
    description: "L’agent lit librement et te demande ton accord avant chaque action. Recommandé.",
  },
  {
    id: "limited",
    label: "Autonome limité",
    description:
      "Les actions à faible risque dans le périmètre d’AiWin11 (ouvrir le Bloc-notes, copier, écrire dans le Bloc-notes qu’il a ouvert) se font sans demander.",
  },
  {
    id: "advanced",
    label: "Avancé",
    description:
      "Les renommages se font aussi sans demander. La corbeille, la lecture de l’écran et le presse-papiers demandent toujours ton accord.",
  },
];

const actionNames: Record<string, string> = {
  inspectProcess: "Détailler un processus",
  findControl: "Chercher un contrôle dans la fenêtre lue",
  launchApplication: "Ouvrir le Bloc-notes",
  listFiles: "Lister les fichiers",
  readFile: "Lire un fichier",
  searchFiles: "Chercher dans les fichiers",
  copyFile: "Copier un fichier",
  moveFile: "Renommer un fichier",
  organizeFiles: "Ranger le dossier (plan de renommages approuvé en une fois)",
  trashFile: "Mettre un fichier à la corbeille",
  inspectWindow: "Lire la structure d’une fenêtre",
  describeWindow: "Décrire une fenêtre (capture + vision locale)",
  typeText: "Écrire dans le Bloc-notes ouvert par l’agent",
  listProcesses: "Lister les processus (lecture seule)",
  systemStatus: "Lire l’état du PC (lecture seule)",
  readClipboard: "Lire le presse-papiers",
  writeClipboard: "Copier un texte dans le presse-papiers",
  listWindows: "Lister les fenêtres ouvertes",
  listServices: "Lister les services Windows (lecture seule)",
  mcpListTools: "Lister les outils d’une extension MCP",
  mcpCall: "Appeler un outil d’une extension MCP",
  generateImage: "Générer une image (moteur local)",
  delegate: "Déléguer à un agent spécialisé",
  runDiagnostic: "Diagnostic Windows de la liste fermée (réseau, Wi-Fi, heure)",
  remember: "Retenir une information (mémoire chiffrée)",
  recall: "Relire la mémoire",
  searchMemory: "Chercher dans la mémoire",
  clickControl: "Cliquer sur un bouton d’une fenêtre lue (UI Automation)",
  setControlValue: "Remplir un champ d’une fenêtre lue (UI Automation)",
  adminService: "Démarrer, arrêter ou configurer un service (administrateur)",
  adminCleanTemp: "Vider les fichiers temporaires de Windows (administrateur)",
  arrangeWindow: "Réduire, restaurer, agrandir ou mettre au premier plan une fenêtre",
};

const effectNames: Record<Effect, string> = {
  allow: "Autorisé",
  ask: "Demande",
  deny: "Refusé",
  simulate: "Simulé",
};

export function Security({ onFinished }: { onFinished: () => void }) {
  const [overview, setOverview] = useState<PolicyOverview>();
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);

  async function load() {
    try {
      setOverview(await invoke<PolicyOverview>("get_policy_overview"));
    } catch {
      setStatus("Les réglages de sécurité sont indisponibles.");
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function save(settings: PolicySettings) {
    setBusy(true);
    try {
      await invoke("set_policy_settings", { settings });
      // The core recomputes the table for the new settings.
      await load();
      setStatus("Réglage enregistré. Il s’applique à la prochaine demande.");
      onFinished();
    } catch {
      setStatus("Le réglage n’a pas pu être enregistré.");
    } finally {
      setBusy(false);
    }
  }

  if (!overview)
    return (
      <section className="card">
        <p role="status">{status || "Chargement…"}</p>
      </section>
    );
  const { settings, table, manifests } = overview;
  const column = profiles.findIndex((profile) => profile.id === settings.autonomy);

  return (
    <section className="card" aria-labelledby="security-heading">
      <h2 id="security-heading">Autonomie de l’agent</h2>
      <fieldset className="task-form" disabled={busy}>
        <legend>Profil</legend>
        {profiles.map((profile) => (
          <label key={profile.id} className="approval">
            <input
              type="radio"
              name="autonomy"
              value={profile.id}
              checked={settings.autonomy === profile.id}
              onChange={() => void save({ ...settings, autonomy: profile.id })}
            />
            <span>
              <strong>{profile.label}</strong> — {profile.description}
            </span>
          </label>
        ))}
        <label className="approval">
          <input
            type="checkbox"
            checked={settings.simulate}
            onChange={(event) => void save({ ...settings, simulate: event.target.checked })}
          />
          <span>
            <strong>Mode simulation</strong> — les actions avec effet ne sont pas exécutées : l’agent te montre ce qu’il
            aurait fait.
          </span>
        </label>
      </fieldset>
      <p role="status">{status}</p>
      <h3>Décisions par action</h3>
      <p>
        Avec un modèle local. Avec OpenAI, seuls l’ouverture du Bloc-notes et les réponses texte sont possibles : aucun
        fichier ni contenu d’écran n’est envoyé au cloud.
      </p>
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              <th scope="col">Action</th>
              {profiles.map((profile, index) => (
                <th key={profile.id} scope="col" aria-current={index === column ? "true" : undefined}>
                  {profile.label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {table.map((row) => (
              <tr key={row.action}>
                <th scope="row">{actionNames[row.action] ?? row.action}</th>
                {row.effects.map((effect, index) => (
                  <td key={index} className={`effect-${effect}`} aria-current={index === column ? "true" : undefined}>
                    {effectNames[effect]}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <PrivacyToggle />
      <AdminToggle />
      <Grants actionNames={actionNames} onChange={onFinished} />
      <h3>Fiche de chaque outil</h3>
      <p>
        Faits vérifiés par AiWin11 pour chaque outil : ils déterminent les règles toujours appliquées (une action
        critique demande toujours ton accord, un outil « local uniquement » n’est jamais proposé à un modèle cloud).
      </p>
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              <th scope="col">Outil</th>
              <th scope="col">Risque</th>
              <th scope="col">Annulation</th>
              <th scope="col">Simulable</th>
              <th scope="col">Local uniquement</th>
              <th scope="col">Délai maximal</th>
            </tr>
          </thead>
          <tbody>
            {manifests.map((manifest) => (
              <tr key={manifest.tool}>
                <th scope="row">
                  {actionNames[manifest.action] ?? manifest.action} <code className="event-code">{manifest.tool}</code>
                </th>
                <td className={`risk-${manifest.risk}`}>{riskNames[manifest.risk]}</td>
                <td>{reversibilityNames[manifest.reversibility]}</td>
                <td>{manifest.sideEffect ? (manifest.dryRun ? "Oui" : "Non") : "—"}</td>
                <td>{manifest.localOnly ? "Oui" : "Non"}</td>
                <td>{Math.round(manifest.timeoutMs / 1000)} s</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
