import { invoke } from "@tauri-apps/api/core";
import { API_URL } from "@/lib/config";

/** The web app this device is paired with — where setup tokens are issued. */
export const APP_URL = process.env.NEXT_PUBLIC_APP_URL ?? "https://aloe.247autoarmy.in";

/** The Connections pane: the one screen that hands out a desktop setup token. */
export const CONNECTIONS_URL = `${APP_URL}/app/settings?section=connections`;

/** Deep link the browser hands the Google OAuth token back through (see the web app's
    sign-in callback page — the Rust side registers this scheme with the OS). */
export const GOOGLE_AUTH_DEEP_LINK = "aloe://auth/callback";

/** Tauri event the Rust side emits when an `aloe://` token arrives while the app is open. */
export const GOOGLE_AUTH_EVENT = "aloe-google-auth";

/** Open a link in the user's real browser rather than inside the app window. */
export const openExternal = (url: string) => invoke<void>("open_external_url", { url });

/**
 * Starts the Google sign-in flow: the backend returns a URL flagged as a desktop flow, and the
 * browser it opens will hand the finished session back through the `aloe://` deep link.
 */
export async function startGoogleAuth(): Promise<void> {
  const response = await fetch(`${API_URL}/api/auth/google?desktop=1`);
  if (!response.ok) {
    throw new Error(`Failed to start Google sign in (${response.status})`);
  }
  const data = (await response.json().catch(() => ({}))) as { url?: string };
  if (!data.url) {
    throw new Error("Failed to start Google sign in: missing url");
  }
  await openExternal(data.url);
}

/** Mints a one-time agent setup token from a signed-in user session, so a fresh Google
    sign-in can pair this device without the user visiting the web app to copy a token. */
export async function mintAgentSetupToken(userToken: string): Promise<string> {
  const response = await fetch(`${API_URL}/api/agent/token`, {
    method: "POST",
    headers: { Authorization: `Bearer ${userToken}` },
  });
  const data = (await response.json().catch(() => ({}))) as { token?: string; error?: string };
  if (!response.ok || !data.token) {
    throw new Error(data.error ?? "Failed to register this device");
  }
  return data.token;
}

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
