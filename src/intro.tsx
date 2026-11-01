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
   theme comes from the same localStorage key the main window's next-themes writes. */

function IntroWindow() {
  const [config, setConfig] = useState<AgentConfig | null>(null);

  useEffect(() => {
    invoke<AgentConfig>("get_config").then(setConfig, () => void getCurrentWindow().close());
  }, []);

  if (!config) return null;
  return <VoiceTour config={config} setConfig={setConfig} onClose={() => void getCurrentWindow().close()} />;
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
