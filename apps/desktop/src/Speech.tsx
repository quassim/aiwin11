import { useEffect, useState } from "react";

const AUTO_KEY = "aiwin11.speech.auto";
const VOICE_KEY = "aiwin11.speech.voice";

/** Local voices only: a voice with `localService === false` would send the text online. */
function localVoices(): SpeechSynthesisVoice[] {
  if (!("speechSynthesis" in window)) return [];
  return window.speechSynthesis.getVoices().filter((voice) => voice.localService);
}

function pickVoice(voices: SpeechSynthesisVoice[]): SpeechSynthesisVoice | undefined {
  const saved = savedVoice();
  if (saved) {
    const selected = voices.find((voice) => voice.voiceURI === saved);
    if (selected) return selected;
  }
  return voices.find((voice) => voice.lang.toLowerCase().startsWith("fr")) ?? voices[0];
}

function savedVoice(): string {
  try {
    return localStorage.getItem(VOICE_KEY) ?? "";
  } catch {
    return "";
  }
}

export function LocalVoiceSetting() {
  const [voices, setVoices] = useState(localVoices);
  const [selection, setSelection] = useState(savedVoice);
  useEffect(() => {
    if (!("speechSynthesis" in window)) return;
    const update = () => setVoices(localVoices());
    window.speechSynthesis.addEventListener("voiceschanged", update);
    return () => window.speechSynthesis.removeEventListener("voiceschanged", update);
  }, []);
  if (!voices.length) return <p>Aucune voix Windows locale détectée.</p>;
  return (
    <label>
      Voix de synthèse locale
      <select
        value={voices.some((voice) => voice.voiceURI === selection) ? selection : ""}
        onChange={(event) => {
          try {
            localStorage.setItem(VOICE_KEY, event.target.value);
            setSelection(event.target.value);
          } catch {
            setSelection(savedVoice());
          }
        }}
      >
        <option value="">Automatique</option>
        {voices.map((voice) => (
          <option key={voice.voiceURI} value={voice.voiceURI}>
            {voice.name} ({voice.lang})
          </option>
        ))}
      </select>
    </label>
  );
}

/** Reads `text` aloud with a local Windows voice. Returns false when none is available. */
export function speak(text: string, onDone?: () => void): boolean {
  const voice = pickVoice(localVoices());
  if (!voice || !text.trim()) return false;
  window.speechSynthesis.cancel();
  const utterance = new SpeechSynthesisUtterance(text);
  utterance.voice = voice;
  utterance.lang = voice.lang;
  if (onDone) {
    utterance.onend = () => onDone();
    utterance.onerror = () => onDone();
  }
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
