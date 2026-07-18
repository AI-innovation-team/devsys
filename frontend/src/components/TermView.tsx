import { FitAddon } from "@xterm/addon-fit";
import { Terminal as XTerm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useCallback, useEffect, useRef, useState } from "react";

import { data } from "../data";
import { Icon } from "../icons";
import { transport, isTauri } from "../transport";
import "../styles/terminal.css";

type KeyDef = { label: string; seq?: string; ctrl?: boolean; paste?: boolean };

// 手机端辅助键条：桌面隐藏，触屏/窄屏自动显示（见 terminal.css）。
// 每键点击即向 PTY 发对应转义序列；粘性 Ctrl 把下一个可打印字符转成控制码。
const KEYS: KeyDef[] = [
  { label: "⇧Tab", seq: "\x1b[Z" },
  { label: "^C", seq: "\x03" },
  { label: "/", seq: "/" },
  { label: "Ctrl", ctrl: true },
  { label: "Tab", seq: "\t" },
  { label: "←", seq: "\x1b[D" },
  { label: "↑", seq: "\x1b[A" },
  { label: "↓", seq: "\x1b[B" },
  { label: "→", seq: "\x1b[C" },
];

// Ctrl+可打印字符 → 控制码（C0）：0x20–0x7e 映射到 char & 0x1f（a/A→\x01 … c/C→\x03 …）。
const ctrlByte = (s: string) => {
  if (s.length !== 1) return s;
  const c = s.charCodeAt(0);
  return c >= 0x20 && c < 0x7f ? String.fromCharCode(c & 0x1f) : s;
};

// 终端核心：一个 xterm ⇄ 一条会话（server, ws）。填满父容器。
// 全屏壳（Terminal.tsx）与工作区 pane 共用这一份实现 —— 后端 Sessions 按 id 支持 N 并发，
// 所以多个 TermView 各开各的会话，互不干扰。
export function TermView({
  server,
  ws,
  embedded,
  onStatus,
  onTitle,
}: {
  server: string;
  ws: string;
  embedded?: boolean;               // 内嵌进 pane：不抢焦点、不显全屏键
  onStatus?: (c: boolean | null) => void; // 连接状态上报给外壳/标签
  onTitle?: (t: string) => void;          // 标题上报（外壳标题栏 / 标签名）
}) {
  const mount = useRef<HTMLDivElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const keys = useRef<HTMLDivElement>(null);
  const sendRef = useRef<((d: string) => void) | null>(null);
  const ctrlRef = useRef(false);
  const [title, setTitle] = useState(ws ? ws : server);
  const [fs, setFs] = useState(false);
  const [ctrl, setCtrl] = useState(false);
  // 回调放 ref：父组件每次渲染都会新建函数，进依赖会重建终端（等于断线重连）。
  const statusRef = useRef(onStatus);
  const titleRef = useRef(onTitle);
  statusRef.current = onStatus;
  titleRef.current = onTitle;

  useEffect(() => {
    data.loadMe().then((me) => {
      const s = me.servers.find((x) => x.name === server);
      if (s) {
        const t = (ws ? ws + "  —  " : "") + (s.username ? s.username + "@" : "") + server + " · " + s.host + ":" + s.port;
        setTitle(t);
        titleRef.current?.(t);
      }
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

    // 传输无关：openTerminal 同步返回句柄；onOpen/onData/onClose 由具体传输回调。
    // 隐藏的 pane（切到别的标签 → display:none）尺寸为 0，跳过 refit，否则 xterm 会算出畸形行列。
    const sendResize = () => {
      const el = box.current;
      if (!el || !el.clientWidth || !el.clientHeight) return;
      try { fit.fit(); } catch { /* ignore */ }
      session.resize(term.cols, term.rows);
    };
    const session = transport.openTerminal(server, ws, {
      onOpen: () => { statusRef.current?.(true); sendResize(); },
      onData: (d) => term.write(d),
      onClose: () => {
        statusRef.current?.(false);
        term.write("\r\n\x1b[2m[AIT.dev] " + (ws ? "已断开 · 工作区仍在后台运行，回门户可重新接入" : "连接已关闭") + "\x1b[0m\r\n");
      },
    });
    const send = (d: string) => session.write(d);
    sendRef.current = send;
    term.onData((d) => {
      // 粘性 Ctrl 激活时，软键盘打出的下一个字符转控制码。
      let out = d;
      if (ctrlRef.current) { ctrlRef.current = false; setCtrl(false); out = ctrlByte(d); }
      send(out);
    });

    // 容器尺寸变化都要 refit —— pane 场景下切标签/分屏不触发 window.resize，
    // 且 display:none→显示 也会让 ResizeObserver 触发（0→实际尺寸）。
    const ro = new ResizeObserver(() => sendResize());
    if (box.current) ro.observe(box.current);
    const onResize = () => sendResize();
    window.addEventListener("resize", onResize);
    const onFs = () => { setFs(!!document.fullscreenElement); setTimeout(() => { sendResize(); term.focus(); }, 80); };
    document.addEventListener("fullscreenchange", onFs);

    return () => {
      ro.disconnect();
      window.removeEventListener("resize", onResize);
      document.removeEventListener("fullscreenchange", onFs);
      session.close();
      term.dispose();
      sendRef.current = null;
    };
  }, [server, ws]);

  const toggleFs = () => {
    if (isTauri) {
      // WKWebView 不支持元素级 requestFullscreen；切 OS 窗口全屏，终端随之填满显示器。
      const w = (window as unknown as { __TAURI__?: any }).__TAURI__?.window?.getCurrentWindow?.();
      if (!w) return;
      w.isFullscreen().then((cur: boolean) => {
        w.setFullscreen(!cur);
        setFs(!cur);
        // 全屏切换后 webview 尺寸变化，稍后 refit 终端。
        setTimeout(() => window.dispatchEvent(new Event("resize")), 200);
      });
      return;
    }
    if (document.fullscreenElement) document.exitFullscreen();
    else box.current?.requestFullscreen?.();
  };

  const press = useCallback((k: KeyDef) => {
    if (k.ctrl) { const n = !ctrlRef.current; ctrlRef.current = n; setCtrl(n); return; }
    if (k.paste) {
      navigator.clipboard?.readText?.().then((txt) => { if (txt) sendRef.current?.(txt); }).catch(() => {});
      if (ctrlRef.current) { ctrlRef.current = false; setCtrl(false); }
      return;
    }
    let seq = k.seq ?? "";
    if (ctrlRef.current) { seq = ctrlByte(seq); ctrlRef.current = false; setCtrl(false); }
    sendRef.current?.(seq);
  }, []);

  // 单行横滑 + 键盘不收：横滑与"touchstart 里 preventDefault"互斥，故用手势判定——
  // touchstart/move 不拦截（放行横向滚动），只在"没滑动的一次轻点"于 touchend 上
  // preventDefault，阻止随后合成的 mouse/focus 序列离开 xterm 的隐藏 textarea，
  // 键盘因此保持。桌面走 mousedown（同样 preventDefault 保焦）。
  useEffect(() => {
    const el = keys.current;
    if (!el) return;
    const btnAt = (t: EventTarget | null) => (t as HTMLElement)?.closest?.(".kbtn") as HTMLElement | null;
    const fire = (btn: HTMLElement | null) => { if (btn) { const k = KEYS[Number(btn.dataset.idx)]; if (k) press(k); } };
    let startBtn: HTMLElement | null = null;
    let sx = 0, sy = 0, moved = false;
    const onTouchStart = (e: TouchEvent) => { const t = e.touches[0]; startBtn = btnAt(e.target); sx = t.clientX; sy = t.clientY; moved = false; };
    const onTouchMove = (e: TouchEvent) => { const t = e.touches[0]; if (Math.abs(t.clientX - sx) > 8 || Math.abs(t.clientY - sy) > 8) moved = true; };
    const onTouchEnd = (e: TouchEvent) => { if (!moved && startBtn) { e.preventDefault(); fire(startBtn); } startBtn = null; };
    const onMouseDown = (e: MouseEvent) => { const b = btnAt(e.target); if (b) { e.preventDefault(); fire(b); } };
    el.addEventListener("touchstart", onTouchStart, { passive: true });
    el.addEventListener("touchmove", onTouchMove, { passive: true });
    el.addEventListener("touchend", onTouchEnd, { passive: false });
    el.addEventListener("mousedown", onMouseDown);
    return () => {
      el.removeEventListener("touchstart", onTouchStart);
      el.removeEventListener("touchmove", onTouchMove);
      el.removeEventListener("touchend", onTouchEnd);
      el.removeEventListener("mousedown", onMouseDown);
    };
  }, [press]);

  return (
    <div className={"term" + (fs ? " fs" : "") + (embedded ? " embedded" : "")} ref={box}>
      <div className="term-title">
        <span className="term-who">{title}</span>
        {!embedded && (
          <button className="fsbtn" onClick={toggleFs} title="全屏"><Icon name={fs ? "min" : "max"} /></button>
        )}
      </div>
      <div className="term-body"><div ref={mount} style={{ height: "100%", width: "100%" }} /></div>
      {/* 不可聚焦 div + 原生 touchstart/mousedown preventDefault（见上方 effect），
          双保险不夺 textarea 焦点、软键盘不收起。data-idx 供事件委托取键。 */}
      <div className="term-keys" role="toolbar" aria-label="辅助键" ref={keys}>
        {KEYS.map((k, i) => (
          <div
            key={k.label}
            role="button"
            data-idx={i}
            className={"kbtn" + (k.ctrl && ctrl ? " active" : "")}
          >{k.label}</div>
        ))}
      </div>
    </div>
  );
}
