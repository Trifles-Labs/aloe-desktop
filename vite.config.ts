import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig(({ command, mode }) => {
  const env = loadEnv(mode, here, "");
  const backendUrl = env.ALOE_BACKEND_URL ?? (command === "serve" ? "http://127.0.0.1:8080" : "https://api.247autoarmy.in");
  const frontendUrl = env.ALOE_FRONTEND_URL ?? (command === "serve" ? "http://localhost:3000" : "https://aloe.247autoarmy.in");

  return {
    plugins: [react(), tailwindcss()],
    resolve: {
      /* One motion runtime for the whole window: if @aloe/ui ever ends up with
         its own framer-motion, the bundle would carry two MotionConfig contexts
         and layout animations that can't see each other across the seam. */
      dedupe: ["react", "react-dom", "framer-motion", "motion-dom", "motion-utils"],
      alias: {
        "react": path.resolve(here, "node_modules/react"),
        "react-dom": path.resolve(here, "node_modules/react-dom"),
        "next/link": path.resolve(here, "src/shims/next-link.tsx"),
        "next/navigation": path.resolve(here, "src/shims/next-navigation.ts"),
        "next/image": path.resolve(here, "src/shims/next-image.tsx"),
      },
    },
    optimizeDeps: {
      include: ["react", "react-dom", "react/jsx-runtime", "react-dom/client"],
      // Shipped as TypeScript source; served as source so the next/* aliases above apply to it.
      exclude: ["@aloe/ui"],
    },
    define: {
      "process.env.NEXT_PUBLIC_API_URL": JSON.stringify(backendUrl),
      "process.env.NEXT_PUBLIC_APP_URL": JSON.stringify(frontendUrl),
    },
    server: {
      strictPort: true,
      port: 1420,
      // ../aloe-ui is where `bun link @aloe/ui` points while developing the shared UI.
      fs: { allow: [here, path.resolve(here, "../aloe-ui")] },
      watch: {
        ignored: [path.resolve(here, "src-tauri/**")],
      },
    },
    build: {
      rollupOptions: {
        input: {
          main: path.resolve(here, "index.html"),
          overlay: path.resolve(here, "overlay.html"),
        },
      },
    },
  };
});
