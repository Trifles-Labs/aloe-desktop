import { invoke } from "@tauri-apps/api/core";

/** The web app this device is paired with — where setup tokens are issued. */
export const APP_URL = process.env.NEXT_PUBLIC_APP_URL ?? "https://aloe.247autoarmy.in";

/** The Connections pane: the one screen that hands out a desktop setup token. */
export const CONNECTIONS_URL = `${APP_URL}/app/settings?section=connections`;

/** Open a link in the user's real browser rather than inside the app window. */
export const openExternal = (url: string) => invoke<void>("open_external_url", { url });

/** Copy, reporting failure rather than throwing — every caller wants a toast. */
export async function copyText(value: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(value);
    return true;
  } catch {
    return false;
  }
}

/** Read the clipboard for the paste shortcut on the setup screen. */
export async function readClipboard(): Promise<string | null> {
  try {
    return await navigator.clipboard.readText();
  } catch {
    return null;
  }
}

export const errorMessage = (cause: unknown) => (cause instanceof Error ? cause.message : String(cause));

/** `folder_sync` → `Folder sync`. Backend kinds are snake_case identifiers. */
export const humanizeKind = (kind: string) => {
  const words = kind.replaceAll("_", " ").trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
};
