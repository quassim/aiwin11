import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

/** Amounts in thousandths of the currency (euros here); 0 means "not set". */
export type CloudCost = { priceMilliPerMillion: number; maxMilliPerTask: number };

const MAX_MILLI = 1_000_000;
const legacyRateKey = "aiwin11-cloud-eur-per-million";

export const euros = (milli: number) =>
  (milli / 1000).toLocaleString("fr-FR", { style: "currency", currency: "EUR", maximumFractionDigits: 3 });

/** Euros typed by the user -> thousandths, or undefined if not a valid amount. */
export function toMilli(text: string): number | undefined {
  if (text.trim() === "") return 0;
  const value = Number(text.replace(",", "."));
  if (!Number.isFinite(value) || value < 0) return undefined;
  const milli = Math.round(value * 1000);
  return milli <= MAX_MILLI ? milli : undefined;
}

const toText = (milli: number) => (milli ? String(milli / 1000) : "");

/** Reads the cost settings; moves a rate kept by older versions in the browser storage once. */
export async function loadCloudCost(): Promise<CloudCost> {
  const cost = await invoke<CloudCost>("get_cloud_cost");
  let legacy: string | null;
  try {
    legacy = localStorage.getItem(legacyRateKey);
  } catch {
    legacy = null;
  }
  if (legacy === null) return cost;
  const price = toMilli(legacy);
  try {
    localStorage.removeItem(legacyRateKey);
  } catch {
    // Nothing to clean up.
  }
  if (cost.priceMilliPerMillion || !price) return cost;
  const migrated = { ...cost, priceMilliPerMillion: price };
  await invoke("set_cloud_cost", { cost: migrated });
  return migrated;
}

export function CloudCostSettings() {
  const [cost, setCost] = useState<CloudCost>();
  const [price, setPrice] = useState("");
  const [max, setMax] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    void loadCloudCost()
      .then((next) => {
        setCost(next);
        setPrice(toText(next.priceMilliPerMillion));
        setMax(toText(next.maxMilliPerTask));
      })
      .catch(() => setError("Réglages de coût indisponibles."));
  }, []);

  function save() {
    const priceMilliPerMillion = toMilli(price);
    const maxMilliPerTask = toMilli(max);
    if (priceMilliPerMillion === undefined || maxMilliPerTask === undefined) {
      setError("Montant invalide : entre 0 et 1 000 €.");
      return;
    }
    const next = { priceMilliPerMillion, maxMilliPerTask };
    if (
      cost &&
      next.priceMilliPerMillion === cost.priceMilliPerMillion &&
      next.maxMilliPerTask === cost.maxMilliPerTask
    )
      return;
    setError("");
    void invoke("set_cloud_cost", { cost: next })
      .then(() => setCost(next))
      .catch(() => setError("Enregistrement impossible."));
  }

  return (
    <fieldset className="task-form" disabled={cost === undefined}>
      <legend>Coût du cloud (tarif saisi par toi, aucun prix intégré)</legend>
      <label htmlFor="cloud-price">Tarif du fournisseur, en euros par million de jetons</label>
      <input
        id="cloud-price"
        inputMode="decimal"
        value={price}
        placeholder="vide = inconnu"
        onChange={(event) => setPrice(event.target.value)}
        onBlur={save}
      />
      <label htmlFor="cloud-max-cost">Coût maximal d’une demande, en euros</label>
      <input
        id="cloud-max-cost"
        inputMode="decimal"
        value={max}
        placeholder="vide = pas de plafond"
        onChange={(event) => setMax(event.target.value)}
        onBlur={save}
      />
      <p>
        Le coût est estimé à partir des jetons comptés et de ce tarif, puis vérifié avant chaque appel cloud ; une fois
        le plafond atteint, la demande s’arrête. Le plafond de jetons reste appliqué.
      </p>
      {error && <p role="alert">{error}</p>}
    </fieldset>
  );
}
