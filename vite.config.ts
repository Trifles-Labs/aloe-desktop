import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const webRoot = path.resolve(here, "../aloe-frontend");

export default defineConfig(({ command }) => ({
  plugins: [react(), tailwindcss()],
  resolve: {
    dedupe: ["react", "react-dom"],
    alias: {
      "@": webRoot,
      "react": path.resolve(here, "node_modules/react"),
      "react-dom": path.resolve(here, "node_modules/react-dom"),
      "next/link": path.resolve(here, "src/shims/next-link.tsx"),
      "next/navigation": path.resolve(here, "src/shims/next-navigation.ts"),
    },
  },
  optimizeDeps: {
    include: ["react", "react-dom", "react/jsx-runtime", "react-dom/client"],
  },
  define: {
    "process.env.NEXT_PUBLIC_API_URL": JSON.stringify(
      process.env.ALOE_BACKEND_URL ?? (command === "serve" ? "http://127.0.0.1:8080" : "https://api.247autoarmy.in"),
    ),
    "process.env.NEXT_PUBLIC_APP_URL": JSON.stringify(
      process.env.ALOE_FRONTEND_URL ?? (command === "serve" ? "http://localhost:3000" : "https://aloe.247autoarmy.in"),
    ),
  },
  server: {
    strictPort: true,
    port: 1420,
    fs: { allow: [here, webRoot] },
  },
  build: {
    rollupOptions: {
      input: {
        main: path.resolve(here, "index.html"),
        orb: path.resolve(here, "orb.html"),
      },
    },
  },
}));
