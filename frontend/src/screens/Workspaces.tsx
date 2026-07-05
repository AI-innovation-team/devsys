import { useCallback, useEffect, useRef, useState } from "react";

import { api, WsServer, WsSession } from "../api";
import { Icon } from "../icons";

const NAME_RE = /^[A-Za-z0-9_.][A-Za-z0-9_.-]{0,63}$/;

function ago(ts: number): string {
  if (!ts) return "";
  const s = Math.floor(Date.now() / 1000) - ts;
  if (s < 60) return "刚刚";
  if (s < 3600) return Math.floor(s / 60) + " 分钟前";
  if (s < 86400) return Math.floor(s / 3600) + " 小时前";
  return Math.floor(s / 86400) + " 天前";
}

export function Workspaces({ goSettings }: { goSettings: () => void }) {
  const [data, setData] = useState<WsServer[] | null>(null);
  const [newSrv, setNewSrv] = useState("");
  const [newName, setNewName] = useState("");
  const [busy, setBusy] = useState(false);
  const [adding, setAdding] = useState(false);
  const [armed, setArmed] = useState<string | null>(null);
  const timer = useRef<number>();
  const nameRef = useRef<HTMLInputElement>(null);
  useEffect(() => { if (adding) nameRef.current?.focus(); }, [adding]);

  const load = useCallback(async () => {
    try { setData((await api.workspaces()).servers); } catch { setData([]); }
  }, []);
  useEffect(() => { load(); }, [load]);

  const servers = data || [];
  const cfg = servers.filter((s) => s.configured && !s.error);
  useEffect(() => {
    if (cfg.length && !cfg.some((s) => s.server === newSrv)) setNewSrv(cfg[0].server);
  }, [data]); // eslint-disable-line react-hooks/exhaustive-deps

  const create = async () => {
    const name = newName.trim();
    if (!NAME_RE.test(name) || !newSrv) return;
    setBusy(true);
    try {
      await api.newWs(newSrv, name);
      window.open(`/terminal/${encodeURIComponent(newSrv)}?ws=${encodeURIComponent(name)}`, "_blank");
      setNewName("");
      setAdding(false);
      await load();
    } catch { /* ignore */ }
    setBusy(false);
  };

  const onKill = (sv: string, name: string) => {
    const k = sv + ":" + name;
    if (armed === k) {
      if (timer.current) clearTimeout(timer.current);
      setArmed(null);
      api.killWs(sv, name).catch(() => {}).then(load);
      return;
    }
    setArmed(k);
    if (timer.current) clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setArmed(null), 2600);
  };

  const rows: { sv: string; ss: WsSession }[] = [];
  servers.forEach((s) => s.sessions.forEach((ss) => rows.push({ sv: s.server, ss })));
  rows.sort((a, b) => Number(b.ss.attached) - Number(a.ss.attached) || b.ss.activity - a.ss.activity);
  const errs = servers.filter((s) => s.error);

  return (
    <div className="wrap">
      <header className="page-head">
        <div className="ph-row">
          <h1>工作区</h1>
          <div style={{ display: "flex", gap: 8 }}>
            <button className="btn subtle sm" onClick={load} title="刷新"><Icon name="refresh" /></button>
            <button className={"btn sm " + (adding ? "subtle" : "primary")} onClick={() => setAdding((a) => !a)} title="新建工作区"><Icon name="plus" /></button>
          </div>
        </div>
      </header>

      {adding && (
        <div className="ws-new-bar">
          {cfg.length ? (
            <div className="ws-new">
              <div className="inp sel"><Icon name="server" /><select value={newSrv} onChange={(e) => setNewSrv(e.target.value)}>{cfg.map((s) => <option key={s.server} value={s.server}>{s.server}</option>)}</select></div>
              <input ref={nameRef} value={newName} onChange={(e) => setNewName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") create(); if (e.key === "Escape") setAdding(false); }} placeholder="名称" maxLength={64} />
              <button className="btn primary sm" disabled={busy} onClick={create}>新建</button>
            </div>
          ) : (
            <span className="nb-label" style={{ color: "var(--text-muted)", fontWeight: 500 }}>无可用服务器 · <a onClick={goSettings}>去设置</a></span>
          )}
        </div>
      )}

      {!data ? null : (
        <>
          {errs.map((s) => <div key={s.server} className="ws-empty err"><Icon name="alert" />{s.server}：{s.error}</div>)}
          {rows.length > 0 ? (
            <div className="ws-list">
              {rows.map(({ sv, ss }) => {
                const k = sv + ":" + ss.name;
                return (
                  <div className="ws-row" key={k}>
                    <div className="ws-info">
                      <span className={"ws-dot" + (ss.attached ? " on" : "")} />
                      <span className="ws-name">{ss.name}</span>
                      <span className="badge accent"><Icon name="server" />{sv}</span>
                      {ss.activity > 0 && <span className="ws-time">{ago(ss.activity)}</span>}
                    </div>
                    <div className="ws-acts">
                      <a className="btn secondary sm" href={`/terminal/${encodeURIComponent(sv)}?ws=${encodeURIComponent(ss.name)}`} target="_blank" rel="noreferrer"><Icon name="terminal" />打开</a>
                      <button className={"btn sm " + (armed === k ? "danger" : "subtle")} title="关闭" onClick={() => onKill(sv, ss.name)}>
                        {armed === k ? "确认" : <Icon name="trash" />}
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          ) : (
            errs.length === 0 && <div className="ws-empty">还没有工作区</div>
          )}
        </>
      )}
    </div>
  );
}
