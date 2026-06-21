import { useEffect, useState } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export function useAutoUpdate() {
    const [updateReady, setUpdateReady] = useState(false);

    useEffect(() => {
        let cancelled = false;
        (async () => {
            try {
                const update = await check();
                if (!update || cancelled) return;
                await update.downloadAndInstall();
                if (!cancelled) setUpdateReady(true);
            } catch {
                // silently ignore — update check failure must not disrupt the app
            }
        })();
        return () => { cancelled = true; };
    }, []);

    const restart = () => relaunch();

    return { updateReady, restart };
}
