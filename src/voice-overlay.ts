import "@fontsource-variable/instrument-sans";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

/* The push-to-talk overlay (voice.html). No React, like overlay.ts: voice/overlay.rs owns the
   state and pushes it in whole with window.eval; this only renders it. */

type Phase = "idle" | "listening" | "transcribing" | "thinking" | "speaking" | "asking" | "done" | "error";
type PromptView = { text: string; options: string[]; hint: string };
type View = {
  phase: Phase;
  transcript: string;
  reply: string;
  status: string | null;
  error: string | null;
  prompt: PromptView | null;
  textSize: string;
  highContrast: boolean;
};

const LABELS: Record<Phase, string> = {
  idle: "",
  listening: "Listening…",
  transcribing: "Listening…",
  thinking: "Aloe is thinking…",
  speaking: "Aloe",
  asking: "Aloe is asking you",
  done: "Aloe",
  error: "Aloe",
};

const root = document.getElementById("root")!;
const label = document.getElementById("label")!;
const said = document.getElementById("said")!;
const reply = document.getElementById("reply")!;
const status = document.getElementById("status")!;
const error = document.getElementById("error")!;
const askText = document.getElementById("ask-text")!;
const askOptions = document.getElementById("ask-options")!;
const askHint = document.getElementById("ask-hint")!;

document.documentElement.dataset.part = getCurrentWindow().label === "voice-panel" ? "panel" : "glow";

/* The reply is markdown; the panel is a glance, not a reader, so it shows the words without the
   syntax. textContent only — nothing from the model is ever parsed as HTML. */
const plain = (markdown: string) =>
  markdown
    .replace(/```[^\n]*\n[\s\S]*?(```|$)/g, "[code]\n")
    .replace(/^\s{0,3}#{1,6}\s+/gm, "")
    .replace(/^\s{0,3}[-*+]\s+/gm, "• ")
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/(\*\*|__|~~|`)/g, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();

const render = (view: View) => {
  // Styling first, so even a fading panel keeps the size and contrast the user chose.
  document.documentElement.dataset.size = view.textSize || "m";
  document.documentElement.dataset.contrast = view.highContrast ? "high" : "normal";
  root.dataset.phase = view.phase;
  if (view.phase === "idle") return; // Keep the old text while it fades out.
  label.textContent = LABELS[view.phase];
  said.textContent = view.transcript;
  reply.textContent = plain(view.reply);
  status.textContent = view.status ?? "";
  error.textContent = view.error ?? "";
  askText.textContent = view.prompt?.text ?? "";
  askHint.textContent = view.prompt?.hint ?? "";
  askOptions.replaceChildren(
    ...(view.prompt?.options ?? []).map((option) => {
      const item = document.createElement("li");
      item.textContent = option;
      return item;
    }),
  );
};

const voiceWindow = window as Window & { __aloeVoice?: (view: View) => void; __aloeVoiceLevel?: (level: number) => void };
voiceWindow.__aloeVoice = render;
voiceWindow.__aloeVoiceLevel = (level) => root.style.setProperty("--level", String(level));
// The window is created by the hold that turns it on, so its first state can land before this
// script runs — ask once.
void invoke<View>("voice_overlay_state").then(render, () => undefined);

document.getElementById("close")?.addEventListener("click", () => {
  root.dataset.phase = "idle";
  void invoke("voice_stop");
});
