/* Where Aloe Code sessions live: files on this computer (src-tauri/src/code_chats.rs), handed to
 * the shared app as Platform.codeStore. The server keeps none of a session; the app sends its
 * history with every turn (@aloe/ui lib/local_code_chats.ts). */

import { invoke } from "@tauri-apps/api/core";

import type { CodeChatStore, LocalCodeChat, LocalCodeChatSummary } from "@aloe/ui/lib/local_code_chats";

export const localCodeStore: CodeChatStore = {
  list: () => invoke<LocalCodeChatSummary[]>("code_chats_list"),
  get: (id) => invoke<LocalCodeChat | null>("code_chat_get", { id }),
  save: (chat) => invoke<void>("code_chat_save", { chat }),
  remove: (id) => invoke<void>("code_chat_delete", { id }),
};
