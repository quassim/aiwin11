import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

export function Updates({ running }: { running: boolean }) {
  const [configured, setConfigured] = useState(false);
  const [version, setVersion] = useState<string>();
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void invoke<boolean>("updates_configured").then(setConfigured, () => setConfigured(false));
  }, []);

  async function check() {
    setBusy(true);
    setVersion(undefined);
    setStatus("Recherche d’une mise à jour…");
    try {
      const available = await invoke<string | null>("check_update");
      setVersion(available ?? undefined);
      setStatus(available ? `Version ${available} disponible.` : "AiWin11 est à jour.");
    } catch {
      setStatus("Le canal de mise à jour est indisponible.");
    } finally {
      setBusy(false);
    }
  }

  async function install() {
    if (!version) return;
    setBusy(true);
    setStatus("Téléchargement et vérification de la signature…");
    try {
      await invoke("install_update", { version });
      setStatus("Installation lancée.");
    } catch (error) {
      setStatus(
        String(error) === "UPDATE_TASK_RUNNING"
          ? "Attends la fin de la tâche avant d’installer la mise à jour."
          : "Installation impossible. Vérifie de nouveau la mise à jour.",
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <section aria-labelledby="updates-title">
      <h2 id="updates-title">Mises à jour</h2>
      {configured ? (
        <div className="section-header">
          <button type="button" disabled={busy} onClick={() => void check()}>
            Rechercher sur GitHub
          </button>
          {version && (
            <button type="button" disabled={busy || running} onClick={() => void install()}>
              Installer la version {version} et fermer AiWin11
            </button>
          )}
        </div>
      ) : (
        <p>Canal de mises à jour signées indisponible dans cette version.</p>
      )}
      <p role="status">{status}</p>
    </section>
  );
}
