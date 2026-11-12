import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type Backup = { name: string; bytes: number };
type Overview = { backups: Backup[]; pendingRestore: string | null };

const reasons: Record<string, string> = {
  manual: "manuelle",
  "before-migration": "avant mise à jour des données",
  "before-restore": "avant restauration",
};

/** "aiwin11-20261113-142530123-manual.sqlite3" -> date, time and reason for display. */
export function describeBackup(name: string): string {
  const match = /^aiwin11-(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})\d*-([a-z-]+)\.sqlite3$/.exec(name);
  if (!match) return name;
  const [, year, month, day, hour, minute, reason] = match;
  return `${day}/${month}/${year} ${hour}:${minute} UTC, ${reasons[reason] ?? reason}`;
}

export function Backups({ disabled }: { disabled: boolean }) {
  const [overview, setOverview] = useState<Overview>();
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);

  async function refresh() {
    try {
      setOverview(await invoke<Overview>("list_backups"));
    } catch {
      setStatus("Sauvegardes indisponibles.");
    }
  }

  useEffect(() => {
    void refresh();
  }, []);

  async function run(action: () => Promise<unknown>, done: string) {
    setBusy(true);
    setStatus("");
    try {
      await action();
      setStatus(done);
    } catch {
      setStatus("L’opération a échoué. Rien n’a été modifié.");
    } finally {
      setBusy(false);
      void refresh();
    }
  }

  return (
    <section aria-labelledby="backups-heading">
      <h2 id="backups-heading">Sauvegardes des données d’AiWin11</h2>
      <p>
        Réglages, historique et mémoire (toujours chiffrée pour ton compte Windows), copiés dans le dossier de données
        local de l’application. Les 5 plus récentes sont gardées ; une copie est faite automatiquement avant chaque mise
        à jour des données.
      </p>
      <button
        type="button"
        disabled={disabled || busy}
        onClick={() => void run(() => invoke("backup_data"), "Sauvegarde créée.")}
      >
        Sauvegarder maintenant
      </button>
      {overview?.pendingRestore && (
        <p role="note">
          Restauration prévue au prochain démarrage : {describeBackup(overview.pendingRestore)}.{" "}
          <button
            type="button"
            disabled={busy}
            onClick={() => void run(() => invoke("cancel_restore"), "Restauration annulée.")}
          >
            Annuler la restauration
          </button>
        </p>
      )}
      {overview && overview.backups.length === 0 && <p>Aucune sauvegarde pour l’instant.</p>}
      {overview && overview.backups.length > 0 && (
        <ul>
          {overview.backups.map((backup) => (
            <li key={backup.name}>
              {describeBackup(backup.name)} ·{" "}
              {(backup.bytes / 1024).toLocaleString("fr-FR", { maximumFractionDigits: 0 })} Kio{" "}
              <button
                type="button"
                disabled={disabled || busy || overview.pendingRestore === backup.name}
                onClick={() =>
                  void run(
                    () => invoke("schedule_restore", { name: backup.name }),
                    "Restauration prévue : redémarre AiWin11 pour l’appliquer. Les données actuelles seront sauvegardées avant.",
                  )
                }
              >
                Restaurer au prochain démarrage
              </button>
            </li>
          ))}
        </ul>
      )}
      <p role="status">{status}</p>
    </section>
  );
}
