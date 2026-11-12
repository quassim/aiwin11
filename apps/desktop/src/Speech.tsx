import { useEffect, useState } from "react";

const AUTO_KEY = "aiwin11.speech.auto";

/** Local voices only: a voice with `localService === false` would send the text online. */
function localVoices(): SpeechSynthesisVoice[] {
  if (!("speechSynthesis" in window)) return [];
  return window.speechSynthesis.getVoices().filter((voice) => voice.localService);
}

function pickVoice(voices: SpeechSynthesisVoice[]): SpeechSynthesisVoice | undefined {
  return voices.find((voice) => voice.lang.toLowerCase().startsWith("fr")) ?? voices[0];
}

/** Reads `text` aloud with a local Windows voice. Returns false when none is available. */
export function speak(text: string): boolean {
  const voice = pickVoice(localVoices());
  if (!voice || !text.trim()) return false;
  window.speechSynthesis.cancel();
  const utterance = new SpeechSynthesisUtterance(text);
  utterance.voice = voice;
  utterance.lang = voice.lang;
  window.speechSynthesis.speak(utterance);
  return true;
}

export function autoSpeak(): boolean {
  try {
    return localStorage.getItem(AUTO_KEY) === "true";
  } catch {
    return false;
  }
}

/** Controls under the agent's reply: read aloud, stop, and read every reply automatically. */
export function SpeechControls({ text }: { text: string }) {
  const [available, setAvailable] = useState(() => localVoices().length > 0);
  const [auto, setAuto] = useState(autoSpeak);
  const [status, setStatus] = useState("");

  useEffect(() => {
    if (!("speechSynthesis" in window)) return;
    const update = () => setAvailable(localVoices().length > 0);
    window.speechSynthesis.addEventListener("voiceschanged", update);
    return () => window.speechSynthesis.removeEventListener("voiceschanged", update);
  }, []);

  if (!available)
    return <p className="hint">Aucune voix Windows locale installée : lecture à voix haute indisponible.</p>;

  return (
    <div className="section-header">
      <button type="button" onClick={() => setStatus(speak(text) ? "" : "Aucune voix locale disponible.")}>
        Lire à voix haute
      </button>
      <button type="button" onClick={() => window.speechSynthesis.cancel()}>
        Arrêter la lecture
      </button>
      <label className="approval">
        <input
          type="checkbox"
          checked={auto}
          onChange={(event) => {
            setAuto(event.target.checked);
            try {
              localStorage.setItem(AUTO_KEY, String(event.target.checked));
            } catch {
              // Not remembered: the choice still applies to this window.
            }
          }}
        />
        Lire chaque réponse automatiquement (voix locale)
      </label>
      <p role="status">{status}</p>
    </div>
  );
}
