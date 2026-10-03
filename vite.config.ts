import { defineConfig, loadEnv, type ViteDevServer } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig(({ command, mode }) => {
  const env = loadEnv(mode, here, "");
  const backendUrl = env.ALOE_BACKEND_URL ?? (command === "serve" ? "http://127.0.0.1:8080" : "https://api.247autoarmy.in");
  const frontendUrl = env.ALOE_FRONTEND_URL ?? (command === "serve" ? "http://localhost:3000" : "https://aloe.247autoarmy.in");

  /* Dev only: errors thrown in the Tauri window (see index.html's dev hook) are echoed into this
     terminal, since a blank window otherwise says nothing about what broke. */
  const echoWindowErrors = {
    name: "aloe-echo-window-errors",
    apply: "serve" as const,
    configureServer(server: ViteDevServer) {
      // Reloads every open window, for when the Tauri window is stuck blank and out of reach.
      server.middlewares.use("/__aloe_reload", (_req, res) => {
        server.ws.send({ type: "full-reload" });
        res.statusCode = 204;
        res.end();
      });
      server.middlewares.use("/__aloe_window_error", (req, res) => {
        let body = "";
        req.on("data", (chunk) => (body += chunk));
        req.on("end", () => {
          console.error(`[window error] ${body}`);
          res.statusCode = 204;
          res.end();
        });
      });
    },
    transformIndexHtml: () => [
      {
        tag: "script",
        injectTo: "head-prepend" as const,
        children: `const send = (message) => { try { fetch("/__aloe_window_error", { method: "POST", body: String(message).slice(0, 4000), keepalive: true }); } catch {} };
window.addEventListener("error", (e) => send(e.error?.stack ?? e.message));
window.addEventListener("unhandledrejection", (e) => send("Unhandled rejection: " + (e.reason?.stack ?? e.reason)));
const error = console.error;
console.error = (...args) => { send(args.map((a) => a?.stack ?? (typeof a === "string" ? a : JSON.stringify(a))).join(" ")); error(...args); };`,
      },
    ],
  };

  return {
    plugins: [react(), tailwindcss(), echoWindowErrors],
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
      include: [
        "react", "react-dom", "react/jsx-runtime", "react-dom/client",
        /* What @aloe/ui imports. Excluding a package also stops Vite from
           scanning its imports, so they are listed here; without pre-bundling,
           react-markdown's CommonJS dependencies (style-to-js) break in dev with
           "does not provide an export named 'default'". Keep in step with
           aloe-ui's package.json. */
        "clsx", "framer-motion", "lucide-react", "next-themes", "qrcode.react",
        "react-icons", "react-icons/fc", "react-icons/pi", "react-icons/si",
        "katex", "katex/contrib/mhchem", "react-markdown", "rehype-katex", "remark-gfm", "remark-math", "tailwind-merge",
      ],
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
