import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// 开发时把 API/WS 代理到本地后端（uvicorn 127.0.0.1:8090）。
// 构建产物进 dist/，由后端托管。
export default defineConfig({
  plugins: [react()],
  // 固定端口 1420（Tauri 惯例，避开常见的 5173 占用）；tauri.conf devUrl 与此一致。
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    proxy: {
      "/api": "http://127.0.0.1:8090",
      "/vscode": "http://127.0.0.1:8090",
      "/ws": { target: "ws://127.0.0.1:8090", ws: true },
    },
  },
  build: { outDir: "dist", chunkSizeWarningLimit: 1500 },
});
