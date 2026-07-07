import { FitAddon } from "@xterm/addon-fit";
import { Terminal as XTerm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef, useState } from "react";

import { api } from "../api";
import { Icon } from "../icons";
import "../styles/terminal.css";

type KeyDef = { label: string; seq?: string; ctrl?: boolean; paste?: boolean };

// 手机端辅助键条：桌面隐藏，触屏/窄屏自动显示（见 terminal.css）。
// 每键点击即向 PTY 发对应转义序列；粘性 Ctrl 把下一个可打印字符转成控制码。
const KEYS: KeyDef[] = [
  { label: "Esc", seq: "\x1b" },
  { label: "Ctrl", ctrl: true },
  { label: "Tab", seq: "\t" },
  { label: "⇧Tab", seq: "\x1b[Z" },
  { label: "←", seq: "\x1b[D" },
  { label: "↑", seq: "\x1b[A" },
  { label: "↓", seq: "\x1b[B" },
  { label: "→", seq: "\x1b[C" },
  { label: "Home", seq: "\x1b[H" },
  { label: "End", seq: "\x1b[F" },
  { label: "/", seq: "/" },
  { label: "-", seq: "-" },
  { label: "|", seq: "|" },
  { label: "~", seq: "~" },
  { label: "^C", seq: "\x03" },
  { label: "^D", seq: "\x04" },
  { label: "粘贴", paste: true },
];

// Ctrl+可打印字符 → 控制码（C0）：0x20–0x7e 映射到 char & 0x1f（a/A→\x01 … c/C→\x03 …）。
const ctrlByte = (s: string) => {
  if (s.length !== 1) return s;
  const c = s.charCodeAt(0);
  return c >= 0x20 && c < 0x7f ? String.fromCharCode(c & 0x1f) : s;
};

export function Terminal({ server, ws }: { server: string; ws: string }) {
  const page = useRef<HTMLDivElement>(null);
  const mount = useRef<HTMLDivElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const sendRef = useRef<((d: string) => void) | null>(null);
  const ctrlRef = useRef(false);
  const [conn, setConn] = useState<null | boolean>(null);
  const [title, setTitle] = useState(ws ? ws : server);
  const [fs, setFs] = useState(false);
  const [ctrl, setCtrl] = useState(false);
  const who = ws ? server + " · " + ws : server;

  useEffect(() => {
    api.me().then((me) => {
      const s = me.servers.find((x) => x.name === server);
      if (s) setTitle((ws ? ws + "  —  " : "") + (s.username ? s.username + "@" : "") + server + " · " + s.host + ":" + s.port);
    }).catch(() => {});

    if (!mount.current) return;
    const term = new XTerm({
      fontSize: 13.5, scrollback: 5000,
      // Symbols 在前（自托管，只含 PUA 图标）；其余按用户本地 Nerd Font → JetBrains Mono → 系统等宽回退。
      fontFamily: '"Symbols Nerd Font Mono", "JetBrainsMono Nerd Font Mono", "JetBrainsMono Nerd Font", "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace',
      cursorBlink: true,
      theme: { background: "#1a1b1e", foreground: "#e6e6e6", cursor: "#7FB069", selectionBackground: "rgba(127,176,105,.28)" },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(mount.current);
    fit.fit();
    term.focus();

    const proto = location.protocol === "https:" ? "wss" : "ws";
    const url = `${proto}://${location.host}/ws/ssh/${encodeURIComponent(server)}` + (ws ? `?ws=${encodeURIComponent(ws)}` : "");
    const sock = new WebSocket(url);
    const send = (d: string) => { if (sock.readyState === 1) sock.send(JSON.stringify({ t: "i", d })); };
    sendRef.current = send;
    const sendResize = () => {
      try { fit.fit(); } catch { /* ignore */ }
      if (sock.readyState === 1) sock.send(JSON.stringify({ t: "r", c: term.cols, r: term.rows }));
    };
    sock.onopen = () => { setConn(true); sendResize(); };
    sock.onmessage = (e) => term.write(e.data);
    sock.onclose = () => {
      setConn(false);
      term.write("\r\n\x1b[2m[AIT.dev] " + (ws ? "已断开 · 工作区仍在后台运行，回门户可重新接入" : "连接已关闭") + "\x1b[0m\r\n");
    };
    term.onData((d) => {
      // 粘性 Ctrl 激活时，软键盘打出的下一个字符转控制码。
      let out = d;
      if (ctrlRef.current) { ctrlRef.current = false; setCtrl(false); out = ctrlByte(d); }
      send(out);
    });

    const onResize = () => sendResize();
    window.addEventListener("resize", onResize);
    const onFs = () => { setFs(!!document.fullscreenElement); setTimeout(() => { sendResize(); term.focus(); }, 80); };
    document.addEventListener("fullscreenchange", onFs);

    return () => {
      window.removeEventListener("resize", onResize);
      document.removeEventListener("fullscreenchange", onFs);
      sock.close();
      term.dispose();
      sendRef.current = null;
    };
  }, [server, ws]);

  // 让辅助键条"贴住软键盘"：web 无原生 inputAccessoryView，改用 VisualViewport ——
  // 键盘弹出会缩小可视视口，把整页高度锁到 vv.height、并按 vv.offsetTop 补偿 iOS 的
  // 视口上滚，键条（在页面底部）便浮在键盘正上方；键盘收起时复位。变化后触发终端 refit。
  useEffect(() => {
    const vv = window.visualViewport;
    const el = page.current;
    if (!vv || !el) return;
    let raf = 0;
    const apply = () => {
      el.style.height = vv.height + "px";
      el.style.transform = `translateY(${vv.offsetTop}px)`;
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => window.dispatchEvent(new Event("resize")));
    };
    vv.addEventListener("resize", apply);
    vv.addEventListener("scroll", apply);
    apply();
    return () => {
      vv.removeEventListener("resize", apply);
      vv.removeEventListener("scroll", apply);
      cancelAnimationFrame(raf);
      el.style.height = "";
      el.style.transform = "";
    };
  }, []);

  const toggleFs = () => {
    if (document.fullscreenElement) document.exitFullscreen();
    else box.current?.requestFullscreen?.();
  };

  const press = (k: KeyDef) => {
    if (k.ctrl) { const n = !ctrlRef.current; ctrlRef.current = n; setCtrl(n); return; }
    if (k.paste) {
      navigator.clipboard?.readText?.().then((txt) => { if (txt) sendRef.current?.(txt); }).catch(() => {});
      if (ctrlRef.current) { ctrlRef.current = false; setCtrl(false); }
      return;
    }
    let seq = k.seq ?? "";
    if (ctrlRef.current) { seq = ctrlByte(seq); ctrlRef.current = false; setCtrl(false); }
    sendRef.current?.(seq);
  };

  return (
    <div className="tpage" ref={page}>
      <div className="tbar">
        <div className="l">
          <a className="back" href="/" title="返回门户"><Icon name="arrowLeft" /></a>
          <span className="tile"><Icon name="terminal" /></span>
          <span className="tbrand">AIT.dev</span>
        </div>
        <span className="sp"><span className={"dot" + (conn === true ? " on" : conn === false ? " off" : "")} />{who}</span>
      </div>
      <div className="stage">
        <div className={"term" + (fs ? " fs" : "")} ref={box}>
          <div className="term-title">
            <span className="term-who">{title}</span>
            <button className="fsbtn" onClick={toggleFs} title="全屏"><Icon name={fs ? "min" : "max"} /></button>
          </div>
          <div className="term-body"><div ref={mount} style={{ height: "100%", width: "100%" }} /></div>
          {/* 用不可聚焦的 div（而非 button）：tap 它不会夺走 xterm 隐藏 textarea 的焦点，
              手机软键盘因此不会被收起；onClick 只在 tap 时触发、横滑滚动键条时不触发。 */}
          <div className="term-keys" role="toolbar" aria-label="辅助键">
            {KEYS.map((k) => (
              <div
                key={k.label}
                role="button"
                className={"kbtn" + (k.ctrl && ctrl ? " active" : "")}
                onClick={() => press(k)}
              >{k.label}</div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
