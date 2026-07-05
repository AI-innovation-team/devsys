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
  const [loading, setLoading] = useState(true);
  const [newSrv, setNewSrv] = useState("");
  const [newName, setNewName] = useState("");
  const [busy, setBusy] = useState(false);
  const [armed, setArmed] = useState<string | null>(null);
  const timer = useRef<number>();

  const load = useCallback(async () => {
    setLoading(true);
    try { setData((await api.workspaces()).servers); } catch { setData([]); }
    setLoading(false);
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
  const uncfg = servers.filter((s) => !s.configured).length;

  return (
    <div className="wrap">
      <header className="page-head">
        <div className="ph-row">
          <div><div className="eyebrow">workspaces</div><h1>工作区</h1></div>
          <button className="btn subtle sm" onClick={load}><Icon name="refresh" />刷新</button>
        </div>
        <p>你的持久会话常驻在服务器上（基于 tmux，与你在机器上 <code>tmux ls</code> 看到的是同一批）。关掉网页只是断开，回来点「打开」即可继续接入 —— 进程与终端历史原样还在。</p>
      </header>

      <div className="ws-new-bar">
        {cfg.length ? (
          <>
            <span className="nb-label">新建工作区</span>
            <div className="ws-new">
              <div className="inp sel"><Icon name="server" /><select value={newSrv} onChange={(e) => setNewSrv(e.target.value)}>{cfg.map((s) => <option key={s.server} value={s.server}>{s.server}</option>)}</select></div>
              <input value={newName} onChange={(e) => setNewName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && create()} placeholder="工作区名（字母/数字）" maxLength={64} />
              <button className="btn primary sm" disabled={busy} onClick={create}><Icon name="plus" />新建</button>
            </div>
          </>
        ) : (
          <span className="nb-label" style={{ color: "var(--text-muted)", fontWeight: 500 }}>
            还没有可用服务器 · <a onClick={goSettings}>前往设置配置凭据</a>
          </span>
        )}
      </div>

      {loading && !data ? (
        <div className="ws-empty">加载中…</div>
      ) : (
        <>
          {errs.map((s) => (
            <div key={s.server} className="ws-empty err"><Icon name="alert" />{s.server} 无法连接：{s.error}</div>
          ))}
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
                      <span className="badge">{ss.windows} 窗口</span>
                      {ss.attached && <span className="badge ok"><Icon name="check" />使用中</span>}
                      {ss.activity > 0 && <span className="ws-time"><Icon name="clock" />活跃 {ago(ss.activity)}</span>}
                    </div>
                    <div className="ws-acts">
                      <a className="btn secondary sm" href={`/terminal/${encodeURIComponent(sv)}?ws=${encodeURIComponent(ss.name)}`} target="_blank" rel="noreferrer"><Icon name="terminal" />打开</a>
                      <button className={"btn sm " + (armed === k ? "danger" : "subtle")} title="关闭工作区" onClick={() => onKill(sv, ss.name)}>
                        {armed === k ? "确认关闭" : <Icon name="trash" />}
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          ) : (
            errs.length === 0 && <div className="ws-empty">还没有工作区 · 在上方选服务器新建一个</div>
          )}
          {uncfg > 0 && (
            <div className="ws-note">{uncfg} 台服务器未配置凭据，不在列表内 · <a onClick={goSettings}>前往设置</a></div>
          )}
        </>
      )}
    </div>
  );
}
