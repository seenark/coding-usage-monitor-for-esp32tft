import tailwindcss from "@tailwindcss/vite";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite-plus";

// Same-origin in production (quota-server serves dist and /api). In dev, proxy to the mock or a real server.
const apiUrl = process.env.API_URL ?? "http://127.0.0.1:8081";

export default defineConfig({
  server: {
    port: 3001,
    proxy: { "/api": apiUrl },
  },
  build: {
    outDir: "../../dist",
    emptyOutDir: true,
  },
  resolve: {
    tsconfigPaths: true,
  },
  plugins: [
    tailwindcss(),
    tanstackRouter({
      target: "react",
      autoCodeSplitting: true,
    }),
    react(),
  ],
});
