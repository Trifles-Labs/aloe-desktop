import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { Leaf } from "lucide-react";
import "./web.css";

const WAKE_WORD_EVENT = "wake-word-detected";
const VOICE_THINKING_EVENT = "voice-thinking";
const VOICE_RESPONSE_EVENT = "voice-response";
const VOICE_ERROR_EVENT = "voice-error";

type OrbState = "idle" | "listening" | "thinking" | "speaking" | "error";

const STATE_GLOW: Record<OrbState, string> = {
  idle: "0 0 28px 4px rgba(111, 135, 71, 0.22)",
  listening: "0 0 60px 12px rgba(125, 184, 232, 0.45)",
  thinking: "0 0 60px 12px rgba(16, 86, 102, 0.45)",
  speaking: "0 0 60px 14px rgba(217, 143, 130, 0.5)",
  error: "0 0 40px 8px rgba(184, 109, 95, 0.45)",
};

const STATE_LABEL: Record<OrbState, string> = {
  idle: "",
  listening: "Listening…",
  thinking: "Thinking…",
  speaking: "Speaking…",
  error: "",
};

// The Web Speech API has no real "gender" field, so this matches common female voice names
// across platforms (Windows/Edge, macOS/Chrome) instead.
const FEMALE_VOICE_HINTS = ["female", "aria", "jenny", "zira", "samantha", "victoria", "susan", "karen", "moira", "tessa", "fiona", "hazel"];

let cachedVoices: SpeechSynthesisVoice[] = [];

function refreshVoices() {
  if ("speechSynthesis" in window) {
    cachedVoices = window.speechSynthesis.getVoices();
  }
}

function pickFemaleVoice(): SpeechSynthesisVoice | undefined {
  if (!cachedVoices.length) refreshVoices();
  const englishVoices = cachedVoices.filter((voice) => voice.lang.toLowerCase().startsWith("en"));
  const pool = englishVoices.length ? englishVoices : cachedVoices;
  return pool.find((voice) => FEMALE_VOICE_HINTS.some((hint) => voice.name.toLowerCase().includes(hint)));
}

function speak(text: string, onDone: () => void) {
  if (!("speechSynthesis" in window)) {
    onDone();
    return;
  }
  window.speechSynthesis.cancel();
  const utterance = new SpeechSynthesisUtterance(text);
  utterance.rate = 1.0;
  const voice = pickFemaleVoice();
  if (voice) utterance.voice = voice;
  utterance.onend = onDone;
  utterance.onerror = onDone;
  window.speechSynthesis.speak(utterance);
}

function Orb() {
  const [state, setState] = useState<OrbState>("idle");
  const hideTimer = useRef<number | null>(null);
  // Set right before an intentional speechSynthesis.cancel() so the interrupted utterance's
  // onerror callback (which fires asynchronously) doesn't clobber the new listening state.
  const interruptedRef = useRef(false);

  useEffect(() => {
    refreshVoices();
    if ("speechSynthesis" in window) {
      window.speechSynthesis.onvoiceschanged = refreshVoices;
    }

    const finishSpeaking = () => {
      if (interruptedRef.current) {
        interruptedRef.current = false;
        return;
      }
      setState("idle");
      hideTimer.current = window.setTimeout(() => void invoke("hide_orb_window"), 1200);
    };

    const unlistenWake = listen(WAKE_WORD_EVENT, () => {
      if (hideTimer.current) window.clearTimeout(hideTimer.current);
      if ("speechSynthesis" in window && window.speechSynthesis.speaking) {
        interruptedRef.current = true;
        window.speechSynthesis.cancel();
      }
      setState("listening");
    });

    const unlistenThinking = listen(VOICE_THINKING_EVENT, () => setState("thinking"));

    const unlistenResponse = listen<string>(VOICE_RESPONSE_EVENT, (event) => {
      setState("speaking");
      speak(event.payload, finishSpeaking);
    });

    const unlistenError = listen<string>(VOICE_ERROR_EVENT, (event) => {
      setState("error");
      speak(event.payload, finishSpeaking);
    });

    return () => {
      void unlistenWake.then((stop) => stop());
      void unlistenThinking.then((stop) => stop());
      void unlistenResponse.then((stop) => stop());
      void unlistenError.then((stop) => stop());
      if (hideTimer.current) window.clearTimeout(hideTimer.current);
    };
  }, []);

  const active = state !== "idle";

  return (
    <div className="flex h-screen w-screen flex-col items-center justify-center gap-3" style={{ background: "transparent" }}>
      <div
        className="liquid-glass flex items-center justify-center rounded-full transition-[box-shadow,transform] duration-500"
        style={{
          width: 140,
          height: 140,
          boxShadow: STATE_GLOW[state],
          transform: active ? "scale(1.04)" : "scale(1)",
        }}
      >
        <Leaf className="h-9 w-9 text-[#6f8747]" style={{ opacity: active ? 0.95 : 0.55 }} />
      </div>
      {STATE_LABEL[state] ? (
        <p className="eyebrow rounded-full bg-white/55 px-3 py-1 text-[#0b3026]" style={{ opacity: 0.85 }}>
          {STATE_LABEL[state]}
        </p>
      ) : null}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Orb />
  </React.StrictMode>,
);
