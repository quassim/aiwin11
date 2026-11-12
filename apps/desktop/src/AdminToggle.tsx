import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

/** Turns the on-demand elevated helper on or off (off by default). */
export function AdminToggle() {
  const [enabled, setEnabled] = useState<boolean>();
  const [status, setStatus] = useState("");

  useEffect(() => {
    void invoke<boolean>("get_admin_helper").then(setEnabled, () => setStatus("Réglage indisponible."));
  }, []);

  async function change(next: boolean) {
    try {
      await invoke("set_admin_helper", { enabled: next });
      setEnabled(next);
      setStatus(next ? "Opérations administrateur activées." : "Opérations administrateur désactivées.");
    } catch {
      setStatus("Le réglage n’a pas pu être enregistré.");
    }
  }

  return (
    <section aria-labelledby="admin-heading">
      <h3 id="admin-heading">Opérations administrateur</h3>
      <label className="approval">
        <input
          type="checkbox"
          checked={enabled ?? false}
          disabled={enabled === undefined}
          onChange={(event) => void change(event.target.checked)}
        />
        <span>
          <strong>Autoriser l’agent à proposer des opérations administrateur</strong> — liste fermée : démarrer,
          arrêter, redémarrer ou changer le démarrage d’un service, vider les fichiers temporaires de Windows. Chaque
          opération demande ton accord puis la confirmation Windows (UAC). Les services de sécurité, de mise à jour et
          les services vitaux sont toujours refusés.
        </span>
      </label>
      <p role="status">{status}</p>
    </section>
  );
}
