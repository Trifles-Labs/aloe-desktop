import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { MotionConfig } from "framer-motion";
import { ThemeProvider } from "next-themes";

import { VoiceTour } from "./components/VoiceIntro";
import type { AgentConfig } from "./types";
import "./web.css";

/* The voice tour's own window (open_voice_intro in lib.rs): frameless, transparent and centred on
   the screen, so it floats over whatever is open instead of living inside the main window. The
   theme comes from the same localStorage key the main window's next-themes writes.

   The window starts hidden and is shown only once the card is drawn: a transparent window still
   takes every click over its whole rectangle, so it must never sit on screen empty. */

const closeWindow = () => void invoke("close_voice_intro");

function IntroWindow() {
  const [config, setConfig] = useState<AgentConfig | null>(null);
  const ready = config !== null;

  useEffect(() => {
    invoke<AgentConfig>("get_config").then(setConfig, closeWindow);
  }, []);

  useEffect(() => {
    if (!ready) return;
    const self = getCurrentWindow();
    void self.show().then(() => self.setFocus());
  }, [ready]);

  if (!config) return null;
  return <VoiceTour config={config} setConfig={setConfig} onClose={closeWindow} />;
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <MotionConfig reducedMotion="user">
      <ThemeProvider attribute="class" defaultTheme="system" enableSystem disableTransitionOnChange>
        <IntroWindow />
      </ThemeProvider>
    </MotionConfig>
  </React.StrictMode>,
);
