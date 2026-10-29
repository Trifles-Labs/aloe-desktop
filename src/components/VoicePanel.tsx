import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Contrast, Gauge, Hand, MessageCircleQuestion, Mic, ShieldAlert, Timer, Type, Volume2, Waves } from "lucide-react";

import Switch from "@aloe/ui/components/ui/Switch";
import { ControlGroup, ControlRow } from "./ControlGroup";
import type { AgentConfig, VoicePrefs } from "../types";

type Props = {
  config: AgentConfig;
  setConfig: (config: AgentConfig) => void;
};

/* Push-to-talk (src-tauri/src/voice). The hotkey is watched from the moment the app starts; the
   switch only decides whether it does anything. The rest is for people who rely on voice: blind
   and low-vision users, and anyone who finds reading or fast speech hard. Native selects and
   sliders throughout, because they are what screen readers and keyboards handle best. */

const CHORD = navigator.userAgent.includes("Mac") ? "Left Control + Left Option" : "Left Ctrl + Left Alt";

const selectClass =
  "h-8 rounded-lg border border-edge bg-surface px-2 text-[13px] text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-moss/40 disabled:opacity-50";

function Choice<T extends string>({ label, value, options, disabled, onChange }: { label: string; value: T; options: Array<[T, string]>; disabled: boolean; onChange: (value: T) => void }) {
  return (
    <select aria-label={label} className={selectClass} value={value} disabled={disabled} onChange={(event) => onChange(event.target.value as T)}>
      {options.map(([option, text]) => (
        <option key={option} value={option}>
          {text}
        </option>
      ))}
    </select>
  );
}

export function VoicePanel({ config, setConfig }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [allowed, setAllowed] = useState(true);
  const prefs = config.voice;

  useEffect(() => {
    invoke<boolean>("voice_hotkey_allowed").then(setAllowed, () => setAllowed(true));
  }, []);

  const save = async (call: Promise<AgentConfig>) => {
    setBusy(true);
    try {
      setConfig(await call);
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };
  const setEnabled = (enabled: boolean) => void save(invoke<AgentConfig>("set_voice_enabled", { enabled }));
  const setPref = <K extends keyof VoicePrefs>(key: K, value: VoicePrefs[K]) =>
    void save(invoke<AgentConfig>("set_voice_prefs", { prefs: { ...prefs, [key]: value } }));

  const mac = config.platform === "macos";
  const off = busy || !config.voiceEnabled;
  const footnote = error ? (
    <span className="text-danger">{error}</span>
  ) : !allowed ? (
    <span className="flex items-start gap-1.5 text-gold">
      <ShieldAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
      {mac
        ? "macOS hasn't let Aloe see the keyboard yet. Allow Aloe Desktop in System Settings › Privacy & Security › Input Monitoring, then restart the app."
        : "Push-to-talk isn't available on this system yet."}
    </span>
  ) : config.voiceEnabled ? (
    `Hold ${CHORD} and talk, or tap it once and just start talking. Aloe reads its answer aloud, asks out loud when it needs something, and only acts when you say yes. Say "say that again", "slower" or "stop" at any time. Press Esc to close the panel.`
  ) : undefined;

  return (
    <ControlGroup label="Voice" footnote={footnote}>
      <ControlRow
        icon={Mic}
        title="Talk to Aloe"
        detail={`Hold or tap ${CHORD} anywhere.`}
        control={<Switch checked={config.voiceEnabled} disabled={busy} label="Talk to Aloe" onChange={setEnabled} />}
      />
      <ControlRow
        icon={Hand}
        title="Tap to talk"
        detail="Tap the keys once instead of holding them. Listening stops by itself when you finish."
        control={<Switch checked={prefs.tapToTalk} disabled={off} label="Tap to talk" onChange={(on) => setPref("tapToTalk", on)} />}
      />
      <ControlRow
        icon={MessageCircleQuestion}
        title="Answer without a key"
        detail="When Aloe asks you something, it listens for your answer straight away."
        control={<Switch checked={prefs.followUps} disabled={off} label="Answer without a key" onChange={(on) => setPref("followUps", on)} />}
      />
      <ControlRow
        icon={Volume2}
        title="Read answers aloud"
        detail="Questions and approvals are always read out."
        control={<Switch checked={prefs.readAloud} disabled={off} label="Read answers aloud" onChange={(on) => setPref("readAloud", on)} />}
      />
      <ControlRow
        icon={Gauge}
        title="Speaking speed"
        control={
          <Choice
            label="Speaking speed"
            value={prefs.pace}
            disabled={off}
            options={[["slow", "Slow"], ["normal", "Normal"], ["fast", "Fast"]]}
            onChange={(pace) => setPref("pace", pace)}
          />
        }
      />
      <ControlRow
        icon={Waves}
        title="Sound cues"
        detail="Short tones when Aloe starts listening, is thinking, or needs your answer."
        control={
          <span className="flex items-center gap-3">
            <input
              type="range"
              min={0}
              max={100}
              step={10}
              aria-label="Sound cue volume"
              className="w-24 accent-moss"
              value={Math.round(prefs.cueVolume * 100)}
              disabled={off || !prefs.soundCues}
              onChange={(event) => setPref("cueVolume", Number(event.target.value) / 100)}
            />
            <Switch checked={prefs.soundCues} disabled={off} label="Sound cues" onChange={(on) => setPref("soundCues", on)} />
          </span>
        }
      />
      <ControlRow
        icon={Type}
        title="Text size"
        detail="On the panel at the bottom of the screen."
        control={
          <Choice
            label="Text size"
            value={prefs.textSize}
            disabled={off}
            options={[["s", "Small"], ["m", "Medium"], ["l", "Large"], ["xl", "Extra large"]]}
            onChange={(size) => setPref("textSize", size)}
          />
        }
      />
      <ControlRow
        icon={Contrast}
        title="High contrast"
        detail="Solid black panel with white text."
        control={<Switch checked={prefs.highContrast} disabled={off} label="High contrast" onChange={(on) => setPref("highContrast", on)} />}
      />
      <ControlRow
        icon={Timer}
        title="Keep the answer on screen"
        control={
          <Choice
            label="Keep the answer on screen"
            value={prefs.linger}
            disabled={off}
            options={[["short", "6 seconds"], ["long", "20 seconds"], ["until_dismissed", "Until I close it"]]}
            onChange={(linger) => setPref("linger", linger)}
          />
        }
      />
    </ControlGroup>
  );
}
