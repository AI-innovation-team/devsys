// Tauri 传输：通过本地 Rust 命令建 SSH 会话。
//   open   → invoke("ssh_open", {server, ws}) 返回 session id
//   write  → invoke("ssh_write", {id, data})
//   resize → invoke("ssh_resize", {id, cols, rows})
//   输出   → event listen("ssh://data/<id>") 写 xterm
//   断连   → event listen("ssh://close/<id>")
//
// 用 tauri.conf 的 app.withGlobalTauri 暴露的 window.__TAURI__ 全局，
// 避免给 web-only 构建引入 @tauri-apps/* 依赖（后续可切换为正式 npm 包）。
import type { TermCallbacks, TermHandle, Transport } from "./index";

type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
type Listen = (
  event: string,
  cb: (e: { payload: unknown }) => void,
) => Promise<() => void>;

const tauri = () => (window as unknown as { __TAURI__: any }).__TAURI__;
const invoke: Invoke = (cmd, args) => tauri().core.invoke(cmd, args);
const listen: Listen = (event, cb) => tauri().event.listen(event, cb);

export const tauriTransport: Transport = {
  openTerminal(server: string, ws: string, cb: TermCallbacks): TermHandle {
    let id: string | null = null;
    let unData: (() => void) | null = null;
    let unClose: (() => void) | null = null;
    let closed = false;

    const fail = () => {
      if (!closed) {
        closed = true;
        cb.onClose();
      }
    };

    (async () => {
      id = (await invoke("ssh_open", { server, ws: ws || null })) as string;
      // Rust 推原始字节（JSON number[]）→ 转 Uint8Array 交给 xterm。
      unData = await listen(`ssh://data/${id}`, (e) =>
        cb.onData(new Uint8Array(e.payload as number[])),
      );
      unClose = await listen(`ssh://close/${id}`, fail);
      cb.onOpen();
    })().catch(fail);

    return {
      write(data: string) {
        if (id) void invoke("ssh_write", { id, data });
      },
      resize(cols: number, rows: number) {
        if (id) void invoke("ssh_resize", { id, cols, rows });
      },
      close() {
        closed = true;
        unData?.();
        unClose?.();
        if (id) void invoke("ssh_close", { id });
      },
    };
  },
};
