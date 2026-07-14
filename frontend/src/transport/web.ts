// Web 传输：包现有门户的 WebSocket 通路（照搬原 Terminal.tsx:66-100 的三消息帧）。
//   输入   {t:"i", d}
//   resize {t:"r", c, r}
//   输出   onmessage → 直接写 xterm（后端推 PTY 原始字节文本）
import type { TermCallbacks, TermHandle, Transport } from "./index";

export const webTransport: Transport = {
  openTerminal(server: string, ws: string, cb: TermCallbacks): TermHandle {
    const proto = location.protocol === "https:" ? "wss" : "ws";
    const url =
      `${proto}://${location.host}/ws/ssh/${encodeURIComponent(server)}` +
      (ws ? `?ws=${encodeURIComponent(ws)}` : "");
    const sock = new WebSocket(url);
    sock.onopen = () => cb.onOpen();
    sock.onmessage = (e) => cb.onData(e.data);
    sock.onclose = () => cb.onClose();

    return {
      write(data: string) {
        if (sock.readyState === 1) sock.send(JSON.stringify({ t: "i", d: data }));
      },
      resize(cols: number, rows: number) {
        if (sock.readyState === 1) sock.send(JSON.stringify({ t: "r", c: cols, r: rows }));
      },
      close() {
        sock.close();
      },
    };
  },
};
