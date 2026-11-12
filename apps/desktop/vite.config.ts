import { defineConfig } from "vite";

export default defineConfig({
  server: { host: "127.0.0.1", port: 1420, strictPort: true },
  esbuild: { jsx: "automatic" },
  build: { rollupOptions: { input: { main: "index.html", overlay: "overlay.html" } } },
});
