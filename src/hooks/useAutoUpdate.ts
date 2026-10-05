import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

// Download and install go through Rust commands (src-tauri/src/updater.rs) rather than the
// updater plugin's JS API: the plugin's install tears the windows down off the main thread on
// Windows, which crashed the app instead of restarting it.
export function useAutoUpdate() {
    const [updateReady, setUpdateReady] = useState(false);

    useEffect(() => {
        let cancelled = false;
        (async () => {
            try {
                const version = await invoke<string | null>("download_update");
                if (version && !cancelled) setUpdateReady(true);
            } catch {
                // silently ignore — update check failure must not disrupt the app
            }
        })();
        return () => { cancelled = true; };
    }, []);

    const restart = () => invoke("install_update");

    return { updateReady, restart };
}
