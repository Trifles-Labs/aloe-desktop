/// <reference types="vite/client" />

declare const process: {
  env: {
    NEXT_PUBLIC_API_URL?: string;
  };
};

interface Window {
  __TAURI_INTERNALS__?: unknown;
}
