import { API_URL } from "@aloe/ui/lib/config";

/* Per-conversation folder access — a folder shared with one chat instead of
   granted to the device permanently. Desktop-only: the path is chosen in the OS
   folder dialog on this machine, so the web app has no use for these calls. */

export type ConversationFolder = {
  id: string;
  path: string;
  label: string | null;
  createdAt: string;
};

type FoldersResponse = { folders?: ConversationFolder[]; connected?: boolean; error?: string };

const foldersUrl = (conversationId: string, grantId?: string) =>
  `${API_URL}/api/agent/conversations/${encodeURIComponent(conversationId)}/folders${grantId ? `/${encodeURIComponent(grantId)}` : ""}`;

async function request(token: string, url: string, init: RequestInit, failure: string): Promise<FoldersResponse> {
  const response = await fetch(url, { ...init, headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" } });
  const data = (await response.json().catch(() => ({}))) as FoldersResponse;
  if (!response.ok) {
    throw new Error(data.error ?? `${failure} (${response.status} ${response.statusText})`);
  }
  return data;
}

const foldersOf = (data: FoldersResponse) => (Array.isArray(data.folders) ? data.folders : []);

// GET /api/agent/conversations/:id/folders
export async function getConversationFolders(token: string, conversationId: string): Promise<{ folders: ConversationFolder[]; connected: boolean }> {
  const data = await request(token, foldersUrl(conversationId), {}, "Failed to load shared folders");
  return { folders: foldersOf(data), connected: Boolean(data.connected) };
}

/**
 * POST /api/agent/conversations/:id/folders — opens the folder dialog on this computer and records
 * whatever the user picks. Slow by design: the request stays open while a person browses their disk.
 */
export async function shareConversationFolder(token: string, conversationId: string): Promise<ConversationFolder[]> {
  return foldersOf(await request(token, foldersUrl(conversationId), { method: "POST" }, "Failed to share a folder with this chat"));
}

// DELETE /api/agent/conversations/:id/folders/:grantId
export async function revokeConversationFolder(token: string, conversationId: string, grantId: string): Promise<ConversationFolder[]> {
  return foldersOf(await request(token, foldersUrl(conversationId, grantId), { method: "DELETE" }, "Failed to remove folder access"));
}
