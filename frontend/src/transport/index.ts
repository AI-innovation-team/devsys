// 传输层抽象：一套前端 UI 同时喂两个目标 ——
//   web 版   → WebSocket/REST 到现有门户后端（backend/devsys_portal）
//   desktop → Tauri invoke/event 到本地 Rust（src-tauri，原生 SSH）
// Terminal.tsx 主体不感知具体传输，只面向下面这个接口。

export interface TermCallbacks {
  onOpen(): void;
  onData(d: string | Uint8Array): void; // web 推文本，tauri 推字节（xterm 两者皆可 write）
  onClose(): void;
}

// 一个已打开的终端会话句柄。write/resize/close 语义与传输无关。
export interface TermHandle {
  write(data: string): void;
  resize(cols: number, rows: number): void;
  close(): void;
}

export interface Transport {
  openTerminal(server: string, ws: string, cb: TermCallbacks): TermHandle;
}

// 运行环境判定：Tauri v2 在 webview 注入 __TAURI_INTERNALS__。
export const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// 同步选择实现。两个实现都无重依赖（tauri 版只用 window 全局，不 import @tauri-apps/*），
// 故静态引入即可，web 构建不受影响。
import { webTransport } from "./web";
import { tauriTransport } from "./tauri";

export const transport: Transport = isTauri ? tauriTransport : webTransport;
