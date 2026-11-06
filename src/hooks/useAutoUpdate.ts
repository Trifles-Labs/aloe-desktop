import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export type ReadyUpdate = {
    version: string;
    currentVersion: string;
    notes: string | null;
};

/* Aloe usually lives in the tray for days, so checking only at launch meant most people never
   heard about an update. Rust returns an already-downloaded update without fetching it again. */
const CHECK_EVERY_MS = 4 * 60 * 60 * 1000;

// Download and install go through Rust commands (src-tauri/src/updater.rs) rather than the
// updater plugin's JS API: the plugin's install tears the windows down off the main thread on
// Windows, which crashed the app instead of restarting it.
export function useAutoUpdate() {
    const [update, setUpdate] = useState<ReadyUpdate | null>(null);
    const [restarting, setRestarting] = useState(false);
    const [restartError, setRestartError] = useState<string | null>(null);

    useEffect(() => {
        let cancelled = false;
        const check = async () => {
            try {
                const ready = await invoke<ReadyUpdate | null>("download_update");
                if (ready && !cancelled) setUpdate(ready);
            } catch {
                // silently ignore — update check failure must not disrupt the app
            }
        };
        void check();
        const timer = window.setInterval(() => void check(), CHECK_EVERY_MS);
        return () => {
            cancelled = true;
            window.clearInterval(timer);
        };
    }, []);

    /* On Windows a successful install never returns: the installer starts and the app exits. */
    const restart = useCallback(async () => {
        setRestarting(true);
        setRestartError(null);
        try {
            await invoke("install_update");
        } catch (err) {
            setRestartError(typeof err === "string" ? err : err instanceof Error ? err.message : "The update couldn't be installed.");
            setRestarting(false);
        }
    }, []);

    return { update, restart, restarting, restartError };
}
