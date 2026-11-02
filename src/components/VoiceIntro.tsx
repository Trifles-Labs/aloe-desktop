import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { ArrowLeft, ArrowRight, Check, Mic, ShieldAlert, X } from "lucide-react";

import Switch from "@aloe/ui/components/ui/Switch";
import { EASE_OUT } from "@aloe/ui/lib/motion";
import { usePathname } from "next/navigation";
import type { AgentConfig } from "../types";

/* The first-run voice tour. Voice is how Aloe Desktop is meant to be used, and a hotkey nobody has
   been told about is invisible (ALOE_VOICE_PLAN.md, "Discoverability"). Four short animated demos
   of the real overlay — hold the keys, it asks before acting, the spoken commands — then a live
   step that lights the keycaps as you press them and finishes when the real overlay starts
   listening (voice/overlay.rs emits "voice-phase" to the tour's window).

   The tour runs in its own floating window centred on the screen (intro.html, open_voice_intro in
   lib.rs). The main window only decides when to open it: once after sign-in and the account
   wizard, and whenever Settings › Desktop app › Voice › Voice tour asks. */

const SEEN_KEY = "aloe_voice_intro_v2";
export const REPLAY_VOICE_INTRO = "aloe:voice-intro";

const seen = () => {
  try {
    return window.localStorage.getItem(SEEN_KEY) === "done";
  } catch {
    return false;
  }
};
const markSeen = () => {
  try {
    window.localStorage.setItem(SEEN_KEY, "done");
  } catch {
    // Without storage the tour may come back next launch; that's all.
  }
};

/* The overlay's own colours (voice.html), so the demos look like the thing they teach. */
const MOSS = "123 189 150";
const MOSS_DEEP = "47 107 79";
const GOLD = "226 196 138";

type Keys = { mod: string; alt: string; modCode: string; altCode: string };
const keysFor = (platform: string): Keys =>
  platform === "macos"
    ? { mod: "control", alt: "option", modCode: "ControlLeft", altCode: "AltLeft" }
    : { mod: "Ctrl", alt: "Alt", modCode: "ControlLeft", altCode: "AltLeft" };

// ── Scripted timelines ────────────────────────────────────────────────────────

/** Steps through `durations` (ms) in a loop while `active`; returns the current frame. */
function useTimeline(durations: number[], active: boolean) {
  const [frame, setFrame] = useState(0);
  useEffect(() => {
    if (!active) return;
    setFrame(0);
    let index = 0;
    let timer = 0;
    const next = () => {
      timer = window.setTimeout(() => {
        index = (index + 1) % durations.length;
        setFrame(index);
        next();
      }, durations[index]);
    };
    next();
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);
  return frame;
}

/** Types `text` out over `ms`, the way the live transcript fills in. */
function Typed({ text, ms }: { text: string; ms: number }) {
  const [count, setCount] = useState(0);
  useEffect(() => {
    setCount(0);
    const step = Math.max(16, ms / text.length);
    const timer = window.setInterval(() => setCount((n) => (n >= text.length ? n : n + 1)), step);
    return () => window.clearInterval(timer);
  }, [text, ms]);
  return (
    <>
      {text.slice(0, count)}
      <span className="opacity-0">{text.slice(count)}</span>
    </>
  );
}

// ── The mock screen ───────────────────────────────────────────────────────────

type IslandPhase = "listening" | "thinking" | "speaking" | "asking" | "working" | "done";

type IslandView = {
  phase: IslandPhase;
  label: string;
  /** The body under the head: what you said, what Aloe says, or what it's asking. */
  body?: ReactNode;
  hint?: string;
};

function Orb({ phase, tint }: { phase: IslandPhase; tint: string }) {
  const listening = phase === "listening";
  const busy = phase === "thinking" || phase === "working";
  return (
    <span
      className="relative grid h-[22px] w-[38px] shrink-0 place-items-center overflow-hidden rounded-full"
      style={{
        background: `radial-gradient(circle at 32% 28%, rgb(255 255 255 / 0.3), transparent 55%), radial-gradient(circle at 50% 60%, rgb(${tint}), rgb(${MOSS_DEEP}))`,
        boxShadow: `0 0 0 1px rgb(255 255 255 / 0.1), 0 0 14px rgb(${tint} / 0.5)`,
        transition: "background 300ms ease",
      }}
    >
      <span className="flex h-full items-center gap-[2px]">
        {[0, 1, 2, 3, 4].map((bar) => (
          <motion.span
            key={bar}
            className="w-[2.5px] rounded-full bg-white/85"
            animate={
              listening || phase === "speaking"
                ? { height: ["25%", `${45 + ((bar * 37) % 40)}%`, "30%", `${55 + ((bar * 23) % 30)}%`, "25%"] }
                : { height: "18%" }
            }
            transition={listening || phase === "speaking" ? { duration: 0.9 + bar * 0.08, repeat: Infinity, ease: "easeInOut" } : { duration: 0.3 }}
          />
        ))}
      </span>
      {busy && (
        <motion.span
          className="absolute inset-0 rounded-full"
          style={{ background: "conic-gradient(transparent 0 60%, rgb(255 255 255 / 0.45) 80%, transparent 95%)" }}
          animate={{ rotate: 360 }}
          transition={{ duration: 1.2, repeat: Infinity, ease: "linear" }}
        />
      )}
      {phase === "asking" && (
        <motion.span
          key="ping"
          className="absolute inset-0 rounded-full"
          style={{ boxShadow: `0 0 0 2px rgb(${GOLD})` }}
          initial={{ opacity: 0.9, scale: 1 }}
          animate={{ opacity: 0, scale: 1.9 }}
          transition={{ duration: 0.9, ease: EASE_OUT }}
        />
      )}
    </span>
  );
}

/** A small desktop with the voice overlay on it: the edge glow and the island at the top. */
function Stage({ view, control = false, children }: { view: IslandView | null; control?: boolean; children?: ReactNode }) {
  const tint = view?.phase === "asking" ? GOLD : MOSS;
  const thinking = view?.phase === "thinking";
  return (
    <div
      className="relative aspect-[16/8] w-full overflow-hidden rounded-2xl"
      style={{ background: "linear-gradient(160deg, #18221c 0%, #0e1511 55%, #121a16 100%)" }}
      aria-hidden="true"
    >
      {/* Wallpaper and two app windows, so the overlay visibly sits over "any app". */}
      <div className="absolute -left-10 -top-16 h-56 w-56 rounded-full opacity-40 blur-3xl" style={{ background: `rgb(${MOSS_DEEP})` }} />
      <div className="absolute -bottom-20 right-0 h-56 w-72 rounded-full opacity-20 blur-3xl" style={{ background: `rgb(${GOLD})` }} />
      <div className="absolute left-[7%] top-[26%] h-[58%] w-[48%] rounded-lg border border-white/10 bg-white/[0.04]">
        <div className="flex h-5 items-center gap-1 border-b border-white/10 px-2">
          {[0, 1, 2].map((dot) => (
            <span key={dot} className="h-1.5 w-1.5 rounded-full bg-white/20" />
          ))}
        </div>
        <div className="grid grid-cols-4 gap-2 p-3">
          {Array.from({ length: 8 }, (_, tile) => (
            <span key={tile} className="aspect-[4/3] rounded bg-white/[0.07]" />
          ))}
        </div>
      </div>
      <div className="absolute right-[7%] top-[36%] h-[46%] w-[34%] rounded-lg border border-white/10 bg-white/[0.035] p-3">
        {[80, 62, 90, 45].map((width, line) => (
          <span key={line} className="mb-2 block h-1.5 rounded-full bg-white/10" style={{ width: `${width}%` }} />
        ))}
      </div>

      {/* The edge glow. */}
      <motion.div
        className="pointer-events-none absolute inset-0 rounded-2xl"
        animate={{
          opacity: view ? 1 : 0,
          boxShadow: control
            ? `inset 0 0 0 3px rgb(${tint}), inset 0 0 40px 8px rgb(${tint} / 0.45)`
            : `inset 0 0 0 2px rgb(${tint} / 0.9), inset 0 0 36px 6px rgb(${tint} / 0.4)`,
        }}
        transition={{ duration: 0.4, ease: EASE_OUT }}
      />
      <motion.div
        className="pointer-events-none absolute inset-0 rounded-2xl"
        style={{ boxShadow: `inset 0 0 90px 24px rgb(${tint} / 0.5)` }}
        animate={view?.phase === "listening" ? { opacity: [0.15, 0.55, 0.25, 0.6, 0.2] } : { opacity: view ? 0.2 : 0 }}
        transition={view?.phase === "listening" ? { duration: 2.2, repeat: Infinity, ease: "easeInOut" } : { duration: 0.4 }}
      />

      {/* The island. */}
      <AnimatePresence>
        {view && (
          <motion.div
            key="island"
            layout
            initial={{ opacity: 0, y: -44, scale: 0.94 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -44, scale: 0.94, transition: { duration: 0.26, ease: [0.55, 0, 0.75, 0.2] } }}
            transition={{ type: "spring", bounce: 0.3, duration: 0.55 }}
            className="absolute left-1/2 top-2.5 w-max min-w-[200px] max-w-[min(360px,calc(100%-24px))] overflow-hidden rounded-[18px] p-[5px] pl-[7px] text-[#f4f7f0]"
            style={{
              x: "-50%",
              background: "rgb(12 17 14 / 0.92)",
              boxShadow: `inset 0 0 0 1px rgb(${tint} / 0.35), 0 12px 32px rgb(0 0 0 / 0.4)`,
            }}
          >
            <motion.div layout="position" className="flex h-[26px] items-center gap-2 pr-1">
              <Orb phase={view.phase} tint={tint} />
              <AnimatePresence mode="popLayout" initial={false}>
                <motion.span
                  key={view.label}
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={{ duration: 0.22, ease: EASE_OUT }}
                  className={`truncate text-[12px] font-medium ${thinking ? "voice-intro-shimmer" : "text-[#f4f7f0]/65"}`}
                >
                  {view.label}
                </motion.span>
              </AnimatePresence>
              {control && (
                <motion.span
                  initial={{ opacity: 0, scale: 0.9 }}
                  animate={{ opacity: 1, scale: 1 }}
                  className="ml-auto rounded-full px-2.5 py-1 text-[11px] font-semibold"
                  style={{ background: `rgb(${GOLD} / 0.2)`, color: `rgb(${GOLD})` }}
                >
                  Stop
                </motion.span>
              )}
            </motion.div>
            <AnimatePresence mode="popLayout" initial={false}>
              {view.body && (
                <motion.div
                  key={view.phase + view.label}
                  layout="position"
                  initial={{ opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.25, ease: EASE_OUT }}
                  className="px-1.5 pb-2 pt-1.5 text-[13px] leading-[1.45]"
                >
                  {view.body}
                  {view.hint && (
                    <span className="mt-1.5 block text-[11px] font-semibold uppercase tracking-[0.08em]" style={{ color: `rgb(${GOLD})` }}>
                      {view.hint}
                    </span>
                  )}
                </motion.div>
              )}
            </AnimatePresence>
          </motion.div>
        )}
      </AnimatePresence>
      {children}
    </div>
  );
}

// ── Keycaps ───────────────────────────────────────────────────────────────────

function Keycap({ label, pressed, size = "md", side = "left" }: { label: string; pressed: boolean; size?: "md" | "lg"; side?: "left" | null }) {
  const lg = size === "lg";
  return (
    <motion.kbd
      animate={{ y: pressed ? 3 : 0 }}
      transition={{ type: "spring", bounce: 0.25, duration: 0.22 }}
      className={`relative inline-flex flex-col items-start justify-between rounded-xl border font-sans not-italic transition-[background-color,border-color,box-shadow,color] duration-150 ${
        lg ? "h-[74px] min-w-[104px] px-3.5 py-2.5" : "h-[52px] min-w-[74px] px-2.5 py-1.5"
      } ${pressed ? "border-moss/70 bg-moss/15 text-ink" : "border-edge bg-surface text-ink-soft"}`}
      style={{
        boxShadow: pressed
          ? `0 1px 0 0 var(--edge), 0 0 0 4px rgb(${MOSS} / 0.18)`
          : "0 4px 0 0 var(--edge), 0 6px 14px rgb(0 0 0 / 0.08)",
      }}
    >
      {side && <span className={`font-medium uppercase tracking-[0.1em] opacity-60 ${lg ? "text-[10px]" : "text-[9px]"}`}>{side}</span>}
      <span className={`font-semibold ${lg ? "text-[18px]" : "text-[14px]"}`}>{label}</span>
    </motion.kbd>
  );
}

function Chord({ keys, pressed, size }: { keys: Keys; pressed: [boolean, boolean]; size?: "md" | "lg" }) {
  return (
    <span className="inline-flex items-center gap-2.5">
      <Keycap label={keys.mod} pressed={pressed[0]} size={size} />
      <span className="text-sm font-semibold text-ink-soft/70">+</span>
      <Keycap label={keys.alt} pressed={pressed[1]} size={size} />
    </span>
  );
}

// ── The demos ─────────────────────────────────────────────────────────────────

const quote = (text: ReactNode) => <span className="text-[#f4f7f0]/80">“{text}”</span>;

function WelcomeDemo({ active }: { active: boolean }) {
  const frame = useTimeline([700, 2400, 1600, 2600, 900], active);
  const views: Array<IslandView | null> = [
    null,
    { phase: "listening", label: "Listening…", body: quote(<Typed text="What's on my calendar this afternoon?" ms={1700} />) },
    { phase: "thinking", label: "Looking at your calendar…" },
    { phase: "speaking", label: "Aloe", body: "Two things: design review at 2, and a call with Priya at 4:30." },
    null,
  ];
  return <Stage view={views[frame]} />;
}

function HoldDemo({ active, keys }: { active: boolean; keys: Keys }) {
  // idle → mod down → both down & talking → let go → thinking → answer
  const frame = useTimeline([900, 250, 2600, 1500, 2600, 700], active);
  const pressed: [boolean, boolean] = [frame === 1 || frame === 2, frame === 2];
  const views: Array<IslandView | null> = [
    null,
    null,
    { phase: "listening", label: "Listening…", body: quote(<Typed text="Rename my screenshots by the date they were taken" ms={2100} />) },
    { phase: "thinking", label: "Thinking…" },
    { phase: "speaking", label: "Aloe", body: "Renamed 18 screenshots on your Desktop, oldest first." },
    null,
  ];
  return (
    <Stage view={views[frame]}>
      <div className="absolute bottom-3 left-1/2 -translate-x-1/2 scale-90">
        <Chord keys={keys} pressed={pressed} />
      </div>
      <AnimatePresence>
        {frame === 3 && (
          <motion.span
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            className="absolute bottom-[74px] left-1/2 -translate-x-1/2 rounded-full bg-black/50 px-2.5 py-1 text-[11px] font-medium text-[#f4f7f0]/80"
          >
            Let go when you're done
          </motion.span>
        )}
      </AnimatePresence>
    </Stage>
  );
}

function ApprovalDemo({ active }: { active: boolean }) {
  const frame = useTimeline([3000, 1300, 1900, 2400, 700], active);
  const views: Array<IslandView | null> = [
    {
      phase: "asking",
      label: "Aloe needs a yes or no",
      body: <span className="text-[15px] font-semibold">Move 18 screenshots into Pictures › 2026?</span>,
      hint: "Say yes or no",
    },
    { phase: "listening", label: "Listening…", body: quote(<Typed text="Yes, go ahead." ms={700} />) },
    { phase: "working", label: "Moving files…" },
    { phase: "done", label: "Done", body: "All 18 are in Pictures › 2026. Anything else?" },
    null,
  ];
  return <Stage view={views[frame]} control={frame === 2} />;
}

const COMMANDS: Array<{ say: string; does: string }> = [
  { say: "Say that again", does: "Repeats the last answer" },
  { say: "Slower", does: "Reads at a gentler pace" },
  { say: "Stop", does: "Stops talking and working" },
];

function CommandsDemo({ active, onFrame }: { active: boolean; onFrame: (frame: number) => void }) {
  const frame = useTimeline([2300, 2300, 2300, 2300], active);
  useEffect(() => onFrame(frame), [frame, onFrame]);
  const command = COMMANDS[frame];
  const view: IslandView | null = command
    ? { phase: "listening", label: command.does, body: quote(command.say) }
    : null;
  return (
    <Stage view={view}>
      <AnimatePresence>
        {frame === 3 && (
          <motion.div
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            className="absolute inset-0 grid place-items-center"
          >
            <span className="flex items-center gap-2 rounded-full bg-black/45 px-3 py-1.5 text-[12px] font-medium text-[#f4f7f0]/85">
              <kbd className="rounded-md border border-white/25 bg-white/10 px-1.5 py-0.5 font-sans text-[11px] font-semibold">Esc</kbd>
              closes the panel
            </span>
          </motion.div>
        )}
      </AnimatePresence>
    </Stage>
  );
}

// ── Live try-it step ──────────────────────────────────────────────────────────

type LiveState = "waiting" | "listening" | "heard";

/** The try-it step's index in the tour's steps. */
const LAST_STEP = 4;

function TryIt({ config, keys, setConfig, live }: { config: AgentConfig; keys: Keys; setConfig: (config: AgentConfig) => void; live: LiveState }) {
  const [held, setHeld] = useState<[boolean, boolean]>([false, false]);
  const [allowed, setAllowed] = useState(true);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    invoke<boolean>("voice_hotkey_allowed").then(setAllowed, () => setAllowed(true));
  }, []);

  /* Mirrors the keys while this window has focus. The global hotkey is what actually opens
     voice; this is only so pressing them feels like it lands somewhere. */
  useEffect(() => {
    const update = (event: KeyboardEvent, down: boolean) => {
      if (event.code === keys.modCode) setHeld(([, alt]) => [down, alt]);
      if (event.code === keys.altCode) {
        setHeld(([mod]) => [mod, down]);
        event.preventDefault(); // Alt alone would focus the (absent) menu bar.
      }
    };
    const down = (event: KeyboardEvent) => update(event, true);
    const up = (event: KeyboardEvent) => update(event, false);
    const blur = () => setHeld([false, false]);
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", blur);
    };
  }, [keys]);

  const enable = async () => {
    setBusy(true);
    try {
      setConfig(await invoke<AgentConfig>("set_voice_enabled", { enabled: true }));
    } finally {
      setBusy(false);
    }
  };

  const ready = config.voiceEnabled && allowed;
  const both = held[0] && held[1];
  const status =
    live === "heard"
      ? "That's it. Aloe works from any app, even when this window is closed."
      : live === "listening"
        ? "Listening — ask it anything, then let go."
        : both
          ? "Keep holding, and start talking…"
          : `Hold left ${keys.mod} + left ${keys.alt}. Try “What's the weather today?”`;

  return (
    <div className="relative grid aspect-[16/8] w-full place-items-center overflow-hidden rounded-2xl border border-edge bg-surface">
      <motion.div
        className="pointer-events-none absolute inset-0 rounded-2xl"
        animate={{
          opacity: live !== "waiting" || both ? 1 : 0,
          boxShadow: `inset 0 0 0 2px rgb(${MOSS} / 0.9), inset 0 0 60px 10px rgb(${MOSS} / ${live === "listening" ? 0.4 : 0.22})`,
        }}
        transition={{ duration: 0.35, ease: EASE_OUT }}
      />
      <div className="relative flex flex-col items-center gap-5 px-6 text-center">
        <AnimatePresence mode="wait" initial={false}>
          {live === "heard" ? (
            <motion.span
              key="done"
              initial={{ scale: 0.6, opacity: 0 }}
              animate={{ scale: 1, opacity: 1 }}
              transition={{ type: "spring", bounce: 0.45, duration: 0.5 }}
              className="grid h-[74px] w-[74px] place-items-center rounded-full bg-moss text-on-accent"
            >
              <Check className="h-9 w-9" strokeWidth={2.5} />
            </motion.span>
          ) : (
            <motion.span key="keys" exit={{ opacity: 0, scale: 0.95 }}>
              <Chord keys={keys} pressed={live === "listening" ? [true, true] : held} size="lg" />
            </motion.span>
          )}
        </AnimatePresence>

        {ready ? (
          <AnimatePresence mode="wait" initial={false}>
            <motion.p
              key={status}
              initial={{ opacity: 0, y: 4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -4 }}
              transition={{ duration: 0.2, ease: EASE_OUT }}
              className="max-w-sm text-[14px] leading-5 text-ink-soft"
              aria-live="polite"
            >
              {status}
            </motion.p>
          </AnimatePresence>
        ) : !allowed ? (
          <p className="flex max-w-sm items-start gap-1.5 text-left text-[13px] leading-5 text-gold">
            <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
            {config.platform === "macos"
              ? "macOS hasn't let Aloe see the keyboard yet. Allow Aloe Desktop in System Settings › Privacy & Security › Input Monitoring, then restart the app."
              : "Push-to-talk isn't available on this system yet."}
          </p>
        ) : (
          <label className="flex items-center gap-3 text-[14px] text-ink">
            Voice is off. Turn it on to try it.
            <Switch checked={false} disabled={busy} label="Talk to Aloe" onChange={() => void enable()} />
          </label>
        )}
      </div>
    </div>
  );
}

// ── Opening the tour (main window) ────────────────────────────────────────────

/* Lives in the main window and opens the tour's own window (open_voice_intro in lib.rs): once on
   first run, after the account wizard and never over it, and again whenever Settings asks. It
   waits until the main window is actually on screen, so a launch into the tray doesn't pop it. */
export function VoiceIntroLauncher({ platform }: { platform: string }) {
  const pathname = usePathname();
  const supported = platform === "windows" || platform === "macos";

  useEffect(() => {
    if (!supported || pathname.startsWith("/app/onboarding") || seen()) return;
    let opening = false;
    const open = () => {
      if (opening || document.hidden || seen()) return;
      opening = true;
      // Seen only once the window really opened, so a failed open tries again next launch.
      invoke("open_voice_intro").then(markSeen, () => (opening = false));
    };
    open();
    document.addEventListener("visibilitychange", open);
    return () => document.removeEventListener("visibilitychange", open);
  }, [supported, pathname]);

  useEffect(() => {
    const replay = () => void invoke("open_voice_intro");
    window.addEventListener(REPLAY_VOICE_INTRO, replay);
    return () => window.removeEventListener(REPLAY_VOICE_INTRO, replay);
  }, []);

  return null;
}

// ── The tour (its own window, intro.html) ─────────────────────────────────────

type Props = {
  config: AgentConfig;
  setConfig: (config: AgentConfig) => void;
  /** Called once the card has animated out; the window closes itself then. */
  onClose: () => void;
};

export function VoiceTour({ config, setConfig, onClose }: Props) {
  const reduceMotion = useReducedMotion();
  const [shown, setShown] = useState(true);
  const [step, setStep] = useState(0);
  const [direction, setDirection] = useState(1);
  const [live, setLive] = useState<LiveState>("waiting");
  const [commandFrame, setCommandFrame] = useState(0);
  const nextRef = useRef<HTMLButtonElement>(null);
  const stepRef = useRef(step);
  stepRef.current = step;
  const closedRef = useRef(false);

  const keys = keysFor(config.platform);

  /* The real overlay reports its phase; the first "listening" on the last step is the win. Using
     voice on an earlier step doesn't count, or the last step would open already finished. */
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen<string>("voice-phase", (event) => {
      if (stepRef.current !== LAST_STEP) return;
      const phase = event.payload;
      setLive((current) => {
        if (current === "heard") return current;
        if (phase === "listening") return "listening";
        if (current === "listening" && phase !== "idle") return "heard";
        if (current === "listening" && phase === "idle") return "waiting";
        return current;
      });
    }).then((stop) => (cancelled ? stop() : (unlisten = stop)));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  /* Closes once, after the exit animation. The timer is the fallback for when the animation never
     finishes (an occluded webview can stop drawing frames), so the window can't outlive the card. */
  const finish = useCallback(() => {
    if (closedRef.current) return;
    closedRef.current = true;
    onClose();
  }, [onClose]);
  const close = useCallback(() => {
    setShown(false);
    window.setTimeout(finish, 400);
  }, [finish]);

  const steps: Array<{ eyebrow: string; title: string; body: ReactNode; demo: ReactNode }> = [
    {
      eyebrow: "Meet voice",
      title: "Talk to your computer. Aloe does the work.",
      body: "Ask out loud from any app. Aloe sees what's on your screen, uses your files, browser and apps, and answers out loud.",
      demo: <WelcomeDemo active={step === 0} />,
    },
    {
      eyebrow: "Push to talk",
      title: `Hold ${keys.mod} + ${keys.alt}, talk, let go.`,
      body: (
        <>
          Use the keys on the <strong className="font-semibold text-ink">left</strong> side of your keyboard. A glow around the screen means
          Aloe is listening; the panel at the top shows what it heard. Rather tap than hold? Turn on <em>Tap to talk</em> in Settings.
        </>
      ),
      demo: <HoldDemo active={step === 1} keys={keys} />,
    },
    {
      eyebrow: "You stay in charge",
      title: "It asks before it acts.",
      body: "Anything that changes your computer is read out first. Just answer “yes” or “no” — no key needed. The screen glows gold while Aloe waits, and Stop is always there while it works.",
      demo: <ApprovalDemo active={step === 2} />,
    },
    {
      eyebrow: "Say it your way",
      title: "Interrupt any time.",
      body: (
        <span className="flex flex-wrap gap-2">
          {[...COMMANDS.map((command) => `“${command.say}”`), "Esc to close"].map((chip, index) => (
            <span
              key={chip}
              className={`rounded-full border px-3 py-1 text-[13px] transition-colors duration-200 ${
                commandFrame === index ? "border-moss/60 bg-moss/12 text-ink" : "border-edge text-ink-soft"
              }`}
            >
              {chip}
            </span>
          ))}
        </span>
      ),
      demo: <CommandsDemo active={step === 3} onFrame={setCommandFrame} />,
    },
    {
      eyebrow: "Your turn",
      title: live === "heard" ? "You're all set." : "Try it now.",
      body: "Press the keys from any app — the Aloe window doesn't need to be open. Voice lives in Settings › Desktop app if you want to change how it sounds or reads.",
      demo: <TryIt config={config} keys={keys} setConfig={setConfig} live={live} />,
    },
  ];

  const last = step === steps.length - 1;
  const go = useCallback(
    (to: number) => {
      if (to < 0) return;
      if (to >= steps.length) return close();
      setDirection(to > step ? 1 : -1);
      setStep(to);
    },
    [close, step, steps.length],
  );

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
      else if (event.key === "ArrowRight") go(step + 1);
      else if (event.key === "ArrowLeft") go(step - 1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [step, go, close]);

  useEffect(() => {
    nextRef.current?.focus({ preventScroll: true });
  }, [step]);

  const current = steps[step];
  const slide = reduceMotion ? 0 : 24;

  return (
    <div className="grid h-screen w-screen place-items-center p-3">
      <AnimatePresence onExitComplete={finish}>
        {shown && (
          <motion.div
            key="card"
            data-tauri-drag-region
            initial={{ opacity: 0, y: 18, scale: 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 10, scale: 0.97, transition: { duration: 0.18, ease: [0.55, 0, 0.75, 0.2] } }}
            transition={{ type: "spring", bounce: 0.22, duration: 0.5 }}
            className="relative flex h-full w-full flex-col overflow-hidden rounded-[24px] border border-edge bg-canvas p-5"
            style={{ boxShadow: "0 24px 60px -12px rgb(0 0 0 / 0.45), 0 2px 6px rgb(0 0 0 / 0.12)" }}
            role="dialog"
            aria-labelledby="voice-intro-title"
          >
            <button
              type="button"
              onClick={close}
              aria-label="Close the voice tour"
              className="absolute right-3 top-3 z-10 grid h-8 w-8 place-items-center rounded-full text-ink-soft transition-colors hover:bg-sage-soft/60 hover:text-ink"
            >
              <X className="h-4 w-4" />
            </button>

            <AnimatePresence mode="wait" initial={false} custom={direction}>
              <motion.div
                key={step}
                data-tauri-drag-region
                className="min-h-0 flex-1"
                initial={{ opacity: 0, x: direction * slide }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: -direction * slide }}
                transition={{ duration: 0.28, ease: EASE_OUT }}
              >
                {current.demo}
                <div data-tauri-drag-region className="mt-5 px-1">
                  <p data-tauri-drag-region className="eyebrow flex items-center gap-1.5">
                    {step === 0 && <Mic className="h-3.5 w-3.5" />}
                    {current.eyebrow}
                  </p>
                  <h2 id="voice-intro-title" data-tauri-drag-region className="mt-1.5 font-display text-[22px] font-semibold leading-tight tracking-[-0.015em] text-ink">
                    {current.title}
                  </h2>
                  <div className="mt-2 text-[14px] leading-[1.55] text-ink-soft">{current.body}</div>
                </div>
              </motion.div>
            </AnimatePresence>

            <div data-tauri-drag-region className="mt-4 flex items-center justify-between gap-3 px-1">
              <div className="flex items-center gap-1.5" role="tablist" aria-label="Tour steps">
                {steps.map((item, index) => (
                  <button
                    key={item.eyebrow}
                    type="button"
                    role="tab"
                    aria-selected={index === step}
                    aria-label={`Step ${index + 1}: ${item.eyebrow}`}
                    onClick={() => go(index)}
                    className="grid h-5 place-items-center"
                  >
                    <motion.span
                      className="block h-1.5 rounded-full"
                      animate={{ width: index === step ? 22 : 6, backgroundColor: index === step ? "var(--moss)" : "var(--edge)" }}
                      transition={{ type: "spring", bounce: 0.2, duration: 0.35 }}
                    />
                  </button>
                ))}
              </div>
              <div className="flex items-center gap-2">
                {step > 0 ? (
                  <button type="button" className="secondary-button min-h-0 px-3.5 py-2 text-sm" onClick={() => go(step - 1)}>
                    <ArrowLeft className="h-4 w-4" />
                    Back
                  </button>
                ) : (
                  <button type="button" className="min-h-0 rounded-lg px-3.5 py-2 text-sm font-semibold text-ink-soft hover:text-ink" onClick={close}>
                    Skip
                  </button>
                )}
                <button ref={nextRef} type="button" className="primary-button min-h-0 px-4 py-2 text-sm" onClick={() => go(step + 1)}>
                  {last ? (live === "heard" ? "Start using Aloe" : "Done") : step === 0 ? "Show me" : "Next"}
                  {!last && <ArrowRight className="h-4 w-4" />}
                </button>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
