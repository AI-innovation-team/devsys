import { useEffect, useState } from "react";

import { data, type TailnetStatus } from "../data";
import { isTauri } from "../transport";
import { Icon } from "../icons";

// 内建 tailnet（tsnet sidecar）控制面板。
// 零系统依赖：把一个 tailnet 节点嵌进 app —— 不装系统 Tailscale。
//   出站：app 内的 SSH 走它经 tailnet 连队友内网机。
//   入站（贡献侧）：把本机 :22 挂上 tailnet，让队友连进来 —— 但 app 得开着。
export function TailnetPanel() {
  const [st, setSt] = useState<TailnetStatus>({ state: "stopped" });
  const [authkey, setAuthkey] = useState("");
  const [ingress, setIngress] = useState(false);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  useEffect(() => {
    if (!isTauri) return;
    data.tailnetStatus().then(setSt).catch(() => {});
    // 订阅 sidecar 状态推送
    const w = window as unknown as { __TAURI__?: { event?: { listen?: (e: string, cb: (ev: { payload: TailnetStatus }) => void) => Promise<() => void> } } };
    let un: (() => void) | undefined;
    w.__TAURI__?.event?.listen?.("tailnet://status", (ev) => setSt(ev.payload)).then((f) => (un = f));
    return () => un?.();
  }, []);

  const up = async () => {
    setBusy(true); setErr("");
    try { await data.tailnetUp(authkey.trim(), ingress); }
    catch (e) { setErr(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };
  const down = async () => {
    setBusy(true); setErr("");
    try { await data.tailnetDown(); }
    catch (e) { setErr(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };

  const running = st.state === "running";
  const connected = running && !!st.ip;
  const needsLogin = running && st.backend === "NeedsLogin";

  return (
    <div className="card"><div className="cfg-body">
      <div className="tn-top">
        <div className={"tn-dot " + (connected ? "ok" : running ? "warn" : "off")} />
        <div className="tn-state">
          <div className="tn-title">
            内建 Tailnet
            {connected && <span className="badge ok">{st.ip}</span>}
            {st.state === "starting" && <span className="badge">启动中…</span>}
            {needsLogin && <span className="badge warn">待登录</span>}
          </div>
          <div className="tn-sub">
            {connected ? `已连接 · ${st.name || ""}${st.ingress ? " · 入站已开（队友可连进来）" : ""}`
              : needsLogin ? "需要登录你的 tailnet"
              : st.state === "starting" ? "正在接入 tailnet…"
              : "未连接 —— 零系统依赖，不装系统 Tailscale"}
          </div>
        </div>
        {running ? (
          <button className="btn subtle sm" disabled={busy} onClick={down}>断开</button>
        ) : (
          <button className="btn primary sm" disabled={busy} onClick={up}><Icon name="network" />{busy ? "连接中…" : "连接"}</button>
        )}
      </div>

      {err && <div className="import-err" style={{ marginTop: 12 }}>{err}</div>}

      {/* 需要浏览器登录：给出 auth URL */}
      {needsLogin && st.auth_url && (
        <div className="acl-note" style={{ marginTop: 12 }}>
          <Icon name="alert" />
          <span>在浏览器打开登录：<a href={st.auth_url} target="_blank" rel="noreferrer">{st.auth_url}</a></span>
        </div>
      )}

      {/* 未连接时的选项 */}
      {!running && (
        <div className="tn-opts">
          <div className="field">
            <label>预授权 key（可选 —— 不填则连接后弹浏览器登录）</label>
            <div className="inp"><Icon name="key" />
              <input value={authkey} onChange={(e) => setAuthkey(e.target.value)} placeholder="tskey-auth-…" autoComplete="off" />
            </div>
          </div>
          <label className="tn-check">
            <input type="checkbox" checked={ingress} onChange={(e) => setIngress(e.target.checked)} />
            <span>
              <strong>开放入站</strong> —— 把本机 SSH 挂上 tailnet，让队友能连进来（贡献算力/跳板）。
              <em>需要本机跑着 sshd；app 关闭则本节点从网络消失。</em>
            </span>
          </label>
        </div>
      )}
    </div></div>
  );
}
