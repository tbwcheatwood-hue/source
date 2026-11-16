import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  server: {
    port: Number(process.env.PORT || 5173),
    strictPort: false,
    proxy: {
      "/rpc": "http://127.0.0.1:7420",
      "/ws": {
        target: "ws://127.0.0.1:7420",
        ws: true,
      },
      "/api": "http://127.0.0.1:7420",
      "/health": "http://127.0.0.1:7420",
      "/mesh": "http://127.0.0.1:7420",
    },
  },
  build: {
    outDir: "dist",
    sourcemap: true,
  },
});
