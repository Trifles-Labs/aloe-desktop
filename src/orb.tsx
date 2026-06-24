import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { motion } from "framer-motion";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { Leaf } from "lucide-react";
import "./web.css";

const WAKE_WORD_EVENT = "wake-word-detected";
const VOICE_THINKING_EVENT = "voice-thinking";
const VOICE_STATUS_EVENT = "voice-status";
const VOICE_RESPONSE_EVENT = "voice-response";
const VOICE_ERROR_EVENT = "voice-error";

type VoicePayload = { text: string; continueListening: boolean; audioBase64?: string };

type OrbState = "idle" | "listening" | "thinking" | "speaking" | "error";

// Brand palette (see aloe-frontend/app/globals.css) — kept consistent with the rest of the
// app's liquid-glass look instead of a literal sci-fi cyan HUD.
const RING_COLOR: Record<OrbState, string> = {
  idle: "111, 135, 71",
  listening: "125, 184, 232",
  thinking: "16, 86, 102",
  speaking: "217, 143, 130",
  error: "184, 109, 95",
};

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

let currentAudio: HTMLAudioElement | null = null;

function voiceDebug(event: string, details: Record<string, unknown> = {}) {
  console.debug("[voice-orb]", event, details);
}

function isSpeaking(): boolean {
  return Boolean(currentAudio) || ("speechSynthesis" in window && window.speechSynthesis.speaking);
}

function stopPlayback() {
  voiceDebug("playback.stop", { hadAudio: Boolean(currentAudio), speechSynthesisSpeaking: "speechSynthesis" in window ? window.speechSynthesis.speaking : false });
  if (currentAudio) {
    currentAudio.onended = null;
    currentAudio.onerror = null;
    currentAudio.pause();
    currentAudio = null;
  }
  if ("speechSynthesis" in window) {
    window.speechSynthesis.cancel();
  }
}

function setVoiceMuted(muted: boolean, reason: string) {
  voiceDebug("native.mute", { muted, reason });
  void invoke("set_voice_muted", { muted }).catch((error) => {
    voiceDebug("native.mute_failed", { muted, reason, error: String(error) });
  });
}

function speak(payload: VoicePayload, onDone: () => void) {
  if (payload.audioBase64) {
    const startedAt = performance.now();
    voiceDebug("playback.audio.start", { textChars: payload.text.length, audioBase64Chars: payload.audioBase64.length, continueListening: payload.continueListening });
    const audio = new Audio(`data:audio/wav;base64,${payload.audioBase64}`);
    currentAudio = audio;
    const finish = () => {
      voiceDebug("playback.audio.done", { elapsedMs: Math.round(performance.now() - startedAt) });
      currentAudio = null;
      onDone();
    };
    audio.onended = finish;
    audio.onerror = finish;
    audio.play().catch(finish);
    return;
  }

  // Fall back to the browser's built-in voice — used for short local status messages
  // (e.g. "I didn't catch a question") that skip the Gemini TTS round-trip entirely.
  if (!("speechSynthesis" in window)) {
    voiceDebug("playback.web_speech.unavailable", { textChars: payload.text.length, continueListening: payload.continueListening });
    onDone();
    return;
  }
  window.speechSynthesis.cancel();
  const utterance = new SpeechSynthesisUtterance(payload.text);
  utterance.rate = 1.0;
  const voice = pickFemaleVoice();
  if (voice) utterance.voice = voice;
  const startedAt = performance.now();
  voiceDebug("playback.web_speech.start", { textChars: payload.text.length, voice: voice?.name ?? null, continueListening: payload.continueListening });
  utterance.onend = () => {
    voiceDebug("playback.web_speech.done", { elapsedMs: Math.round(performance.now() - startedAt) });
    onDone();
  };
  utterance.onerror = (event) => {
    voiceDebug("playback.web_speech.error", { error: event.error, elapsedMs: Math.round(performance.now() - startedAt) });
    onDone();
  };
  window.speechSynthesis.speak(utterance);
}

/** A rotating ring with a comet-like gradient sweep — the core "Jarvis HUD" visual. */
function EnergyRing({
  size,
  color,
  opacity,
  duration,
  direction = 1,
  thickness = 2,
}: {
  size: number;
  color: string;
  opacity: number;
  duration: number;
  direction?: 1 | -1;
  thickness?: number;
}) {
  const maskImage = `radial-gradient(closest-side, transparent calc(100% - ${thickness + 2}px), black calc(100% - ${thickness}px))`;
  return (
    <motion.div
      className="absolute rounded-full"
      style={{
        width: size,
        height: size,
        left: "50%",
        top: "50%",
        marginLeft: -size / 2,
        marginTop: -size / 2,
        opacity,
        background: `conic-gradient(from 0deg, transparent 0%, rgba(${color}, 0.9) 12%, transparent 30%, transparent 70%, rgba(${color}, 0.6) 88%, transparent 100%)`,
        WebkitMaskImage: maskImage,
        maskImage,
        transition: "opacity 600ms ease",
      }}
      animate={{ rotate: 360 * direction }}
      transition={{ duration, repeat: Infinity, ease: "linear" }}
    />
  );
}

/** A small light orbiting the core, like a satellite tracking around the orb. */
function OrbitingSpark({ radius, color, duration, opacity }: { radius: number; color: string; duration: number; opacity: number }) {
  return (
    <motion.div
      className="absolute left-1/2 top-1/2"
      style={{ width: radius * 2, height: radius * 2, marginLeft: -radius, marginTop: -radius, opacity, transition: "opacity 600ms ease" }}
      animate={{ rotate: 360 }}
      transition={{ duration, repeat: Infinity, ease: "linear" }}
    >
      <div
        className="absolute rounded-full"
        style={{ width: 7, height: 7, top: 0, left: "50%", marginLeft: -3.5, background: `rgb(${color})`, boxShadow: `0 0 10px 3px rgba(${color}, 0.8)` }}
      />
    </motion.div>
  );
}

function Orb() {
  const [state, setState] = useState<OrbState>("idle");
  const [statusText, setStatusText] = useState<string | null>(null);
  const stateRef = useRef<OrbState>("idle");
  const hideTimer = useRef<number | null>(null);
  const speechTokenRef = useRef(0);

  useEffect(() => {
    const setOrbState = (nextState: OrbState) => {
      voiceDebug("state", { from: stateRef.current, to: nextState });
      stateRef.current = nextState;
      setState(nextState);
    };

    refreshVoices();
    if ("speechSynthesis" in window) {
      window.speechSynthesis.onvoiceschanged = refreshVoices;
    }

    const finishSpeaking = (continueListening: boolean) => {
      setVoiceMuted(false, "speech_finished");
      if (continueListening) {
        voiceDebug("listening.resume_after_speech");
        void invoke("resume_voice_listening");
        setOrbState("listening");
        return;
      }
      setOrbState("idle");
      hideTimer.current = window.setTimeout(() => void invoke("hide_orb_window"), 1200);
    };

    const stopTurn = () => {
      voiceDebug("turn.stop_requested");
      if (hideTimer.current) window.clearTimeout(hideTimer.current);
      speechTokenRef.current += 1;
      stopPlayback();
      setVoiceMuted(false, "turn_stopped");
      setStatusText(null);
      setOrbState("idle");
      void invoke("stop_current_turn");
    };

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      stopTurn();
    };

    document.addEventListener("keydown", onKeyDown, true);

    const unlistenWake = listen(WAKE_WORD_EVENT, () => {
      voiceDebug("event.wake_word", { wasSpeaking: isSpeaking() });
      if (hideTimer.current) window.clearTimeout(hideTimer.current);
      if (isSpeaking()) {
        speechTokenRef.current += 1;
        stopPlayback();
      }
      setStatusText(null);
      setOrbState("listening");
    });

    const unlistenThinking = listen(VOICE_THINKING_EVENT, () => {
      if (stateRef.current === "speaking") return;
      voiceDebug("event.thinking");
      setStatusText(null);
      setOrbState("thinking");
    });

    const unlistenStatus = listen<string>(VOICE_STATUS_EVENT, (event) => {
      if (stateRef.current === "speaking") return;
      voiceDebug("event.status", { status: event.payload });
      setStatusText(event.payload);
    });

    const unlistenResponse = listen<VoicePayload>(VOICE_RESPONSE_EVENT, (event) => {
      voiceDebug("event.response", { textChars: event.payload.text.length, continueListening: event.payload.continueListening, hasAudio: Boolean(event.payload.audioBase64) });
      const speechToken = speechTokenRef.current + 1;
      speechTokenRef.current = speechToken;
      setStatusText(null);
      setOrbState("speaking");
      setVoiceMuted(true, "speech_started");
      speak(event.payload, () => {
        if (speechTokenRef.current === speechToken) {
          finishSpeaking(event.payload.continueListening);
        }
      });
    });

    const unlistenError = listen<VoicePayload>(VOICE_ERROR_EVENT, (event) => {
      voiceDebug("event.error", { text: event.payload.text, continueListening: event.payload.continueListening, hasAudio: Boolean(event.payload.audioBase64) });
      const speechToken = speechTokenRef.current + 1;
      speechTokenRef.current = speechToken;
      setStatusText(null);
      setOrbState("error");
      setVoiceMuted(true, "error_speech_started");
      speak(event.payload, () => {
        if (speechTokenRef.current === speechToken) {
          finishSpeaking(event.payload.continueListening);
        }
      });
    });

    return () => {
      void unlistenWake.then((stop) => stop());
      void unlistenThinking.then((stop) => stop());
      void unlistenStatus.then((stop) => stop());
      void unlistenResponse.then((stop) => stop());
      void unlistenError.then((stop) => stop());
      if (hideTimer.current) window.clearTimeout(hideTimer.current);
      setVoiceMuted(false, "orb_unmount");
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, []);

  const active = state !== "idle";
  const color = RING_COLOR[state];
  const label = state === "thinking" && statusText ? statusText : STATE_LABEL[state];

  return (
    <div className="flex h-screen w-screen flex-col items-center justify-center gap-4" style={{ background: "transparent" }}>
      <div className="relative" style={{ width: 360, height: 360 }}>
        <EnergyRing size={360} color={color} opacity={active ? 0.65 : 0.18} duration={active ? 7 : 18} direction={1} thickness={2} />
        <EnergyRing size={280} color={color} opacity={active ? 0.55 : 0.14} duration={active ? 10 : 22} direction={-1} thickness={2} />
        <EnergyRing size={205} color={color} opacity={active ? 0.6 : 0.12} duration={active ? 4.5 : 14} direction={1} thickness={3} />
        <OrbitingSpark radius={140} color={color} duration={active ? 3 : 11} opacity={active ? 0.9 : 0.25} />

        <motion.div
          className="liquid-glass absolute left-1/2 top-1/2 flex items-center justify-center rounded-full"
          style={{ width: 140, height: 140, marginLeft: -70, marginTop: -70, boxShadow: STATE_GLOW[state] }}
          animate={{ scale: active ? [1, 1.045, 1] : 1 }}
          transition={{ duration: 1.8, repeat: active ? Infinity : 0, ease: "easeInOut" }}
        >
          <Leaf className="h-9 w-9 text-[#6f8747]" style={{ opacity: active ? 0.95 : 0.55 }} />
        </motion.div>
      </div>
      {label ? (
        <p className="eyebrow max-w-75 truncate rounded-full bg-white/55 px-3 py-1 text-[#0b3026]" style={{ opacity: 0.85 }}>
          {label}
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
