import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

const applications = [
  { code: "notepad", name: "Bloc-notes" },
  { code: "charmap", name: "Table des caractères" },
  { code: "paint", name: "Paint" },
] as const;

/** Applications of the closed list the agent may not open, whatever the profile. */
export function BlockedApps() {
  const [blocked, setBlocked] = useState<string[]>();
  const [status, setStatus] = useState("");

  useEffect(() => {
    void invoke<string[]>("get_blocked_applications")
      .then(setBlocked)
      .catch(() => setStatus("Règles indisponibles."));
  }, []);

  function toggle(code: string, allowed: boolean) {
    if (!blocked) return;
    const next = allowed ? blocked.filter((item) => item !== code) : [...blocked, code];
    void invoke("set_blocked_applications", { applications: next })
      .then(() => {
        setBlocked(next);
        setStatus("Règle enregistrée.");
      })
      .catch(() => setStatus("Enregistrement impossible."));
  }

  return (
    <section className="card" aria-labelledby="blocked-apps-heading">
      <h2 id="blocked-apps-heading">Applications que l’agent peut ouvrir</h2>
      <p>
        Décoche une application pour l’interdire à l’agent dans tous les profils : il ne te demandera même pas l’accord,
        et le broker refuse l’ouverture au moment de l’exécution.
      </p>
      {applications.map(({ code, name }) => (
        <label key={code} className="approval">
          <input
            type="checkbox"
            disabled={blocked === undefined}
            checked={blocked !== undefined && !blocked.includes(code)}
            onChange={(event) => toggle(code, event.target.checked)}
          />{" "}
          {name}
        </label>
      ))}
      <p role="status">{status}</p>
    </section>
  );
}
