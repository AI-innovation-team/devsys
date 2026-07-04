import { FitAddon } from "@xterm/addon-fit";
import { Terminal as XTerm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef, useState } from "react";

import { api } from "../api";
import { Icon } from "../icons";
import "../styles/terminal.css";

export function Terminal({ server, ws }: { server: string; ws: string }) {
  const mount = useRef<HTMLDivElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const [conn, setConn] = useState<null | boolean>(null);
  const [title, setTitle] = useState(ws ? ws : server);
  const [fs, setFs] = useState(false);
  const who = ws ? server + " · " + ws : server;

  useEffect(() => {
    api.me().then((me) => {
      const s = me.servers.find((x) => x.name === server);
      if (s) setTitle((ws ? ws + "  —  " : "") + (s.username ? s.username + "@" : "") + server + " · " + s.host + ":" + s.port);
    }).catch(() => {});

    if (!mount.current) return;
    const term = new XTerm({
      fontSize: 13.5, scrollback: 5000,
      fontFamily: '"JetBrains Mono",ui-monospace,SFMono-Regular,Menlo,monospace',
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
    const sendResize = () => {
      try { fit.fit(); } catch { /* ignore */ }
      if (sock.readyState === 1) sock.send(JSON.stringify({ t: "r", c: term.cols, r: term.rows }));
    };
    sock.onopen = () => { setConn(true); sendResize(); };
    sock.onmessage = (e) => term.write(e.data);
    sock.onclose = () => {
      setConn(false);
      term.write("\r\n\x1b[2m[devsys] " + (ws ? "已断开 · 工作区仍在后台运行，回门户可重新接入" : "连接已关闭") + "\x1b[0m\r\n");
    };
    term.onData((d) => { if (sock.readyState === 1) sock.send(JSON.stringify({ t: "i", d })); });

    const onResize = () => sendResize();
    window.addEventListener("resize", onResize);
    const onFs = () => { setFs(!!document.fullscreenElement); setTimeout(() => { sendResize(); term.focus(); }, 80); };
    document.addEventListener("fullscreenchange", onFs);

    return () => {
      window.removeEventListener("resize", onResize);
      document.removeEventListener("fullscreenchange", onFs);
      sock.close();
      term.dispose();
    };
  }, [server, ws]);

  const toggleFs = () => {
    if (document.fullscreenElement) document.exitFullscreen();
    else box.current?.requestFullscreen?.();
  };

  return (
    <div className="tpage">
      <div className="tbar">
        <div className="l">
          <a className="back" href="/" title="返回门户"><Icon name="arrowLeft" /></a>
          <span className="tile"><Icon name="terminal" /></span>
          <span className="tbrand">devsys</span>
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
        </div>
      </div>
    </div>
  );
}
