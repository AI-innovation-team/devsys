import { useEffect, useRef, useState } from "react";

import { data, type TailnetStatus } from "../data";
import { isTauri } from "../transport";
import { Icon } from "../icons";

// 内建 tailnet（tsnet sidecar）控制面板。
// 零系统依赖：把一个 tailnet 节点嵌进 app —— 不装系统 Tailscale。
//   出站：app 内的 SSH 走它经 tailnet 连队友内网机。
//   入站（贡献侧）：把本机 :22 挂上 tailnet，让队友连进来 —— 但 app 得开着。
export function TailnetPanel({ teamTailnet, teamPath }: { teamTailnet?: string; teamPath?: string }) {
  const [st, setSt] = useState<TailnetStatus>({ state: "stopped" });
  // 调用方没直接给团队 tailnet 时，自己按 teamPath 去读 —— 少一个「忘了传」的坑。
  const [fetched, setFetched] = useState<string>("");
  const [log, setLog] = useState<string[]>([]);
  const [showLog, setShowLog] = useState(false);
  const [slow, setSlow] = useState(false);
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

  // 团队声明的 tailnet 若是 URL(http/https)= 自建 Headscale 控制面,接入时指向它;
  // 否则(官方 Tailscale 的 tailnet 名)control 留空,连官方网。
  const teamNet = teamTailnet || fetched;
  const control = teamNet && /^https?:\/\//i.test(teamNet) ? teamNet : "";
  // OIDC 入网:控制面(Headscale+Dex)要求登录时,tsnet 报 auth_url →
  // **自动弹浏览器**,成员只需在浏览器点一下 GitHub 授权即入网(零配置、身份到人)。
  const opened = useRef("");
  useEffect(() => {
    const u = st.auth_url;
    if (u && opened.current !== u) {
      opened.current = u;
      data.openUrl(u).catch(() => {}); // 弹不出也无妨,下方仍给可点链接
    }
  }, [st.auth_url]);

  useEffect(() => {
    if (!isTauri || teamTailnet || !teamPath) return;
    data.readTeamView(teamPath).then((v) => setFetched(v.tailnet || "")).catch(() => {});
  }, [teamPath, teamTailnet]);

  // 接入超过 20 秒还没成 = 多半不是「慢」，是卡住了。把 helper 日志摊开，
  // 别让人对着转圈干等（这正是「一直在连接」那次我们两眼一抹黑的原因）。
  useEffect(() => {
    if (st.state !== "starting") { setSlow(false); return; }
    const t = setTimeout(() => {
      setSlow(true);
      data.tailnetLog().then(setLog).catch(() => {});
    }, 20000);
    return () => clearTimeout(t);
  }, [st.state]);

  const refreshLog = () => { data.tailnetLog().then(setLog).catch(() => {}); setShowLog(true); };

  const up = async () => {
    setBusy(true); setErr("");
    try { await data.tailnetUp(authkey.trim(), ingress, control); }
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

      {/* 团队声明的 tailnet：成员据此确认加入的是同一张网（可达性地基）。 */}
      {teamTailnet && (
        <div className="acl-note" style={{ marginTop: 12 }}>
          <Icon name="network" />
          <span>
            {control
              ? <>团队自持网(Headscale)<strong>{teamTailnet}</strong> —— 「连接」即接入这张网,不走官方 Tailscale。</>
              : <>你的团队在 tailnet <strong>{teamTailnet}</strong> —— 登录时确认进的是这张网,大家才互相可达。</>}
          </span>
        </div>
      )}

      {err && <div className="import-err" style={{ marginTop: 12 }}>{err}</div>}
      {/* helper 自己报的错，以前根本没显示过 */}
      {st.error && <div className="import-err" style={{ marginTop: 12 }}>{st.error}</div>}
      {/* 我们替用户读日志得出的结论 —— 比原始日志有用得多 */}
      {st.hint && (
        <div className="import-err" style={{ marginTop: 12 }}>
          <strong>接不进去：</strong>{st.hint}
        </div>
      )}
      {slow && !st.hint && (
        <div className="acl-note" style={{ marginTop: 12 }}>
          <Icon name="alert" />
          <span>
            接入已经超过 20 秒还没成 —— 多半是卡住了，不是慢。
            {!control && <> 而且这次<strong>没带团队控制面地址</strong>，连的是官方 Tailscale。</>}
            <button className="org-authlink" onClick={refreshLog}>看 helper 日志 →</button>
          </span>
        </div>
      )}
      {showLog && (
        <pre className="acl-json" style={{ marginTop: 12 }}>
          {log.length ? log.join("\n") : "（helper 还没输出任何日志）"}
        </pre>
      )}

      {/* 需要浏览器登录：给出 auth URL */}
      {needsLogin && st.auth_url && (
        <div className="acl-note" style={{ marginTop: 12 }}>
          <Icon name="alert" />
          <span>
            已自动打开浏览器 —— 点 <strong>Log in with GitHub</strong> 授权即入网。
            没弹出?<button className="org-authlink" onClick={() => data.openUrl(st.auth_url!).catch(() => {})}>重新打开 →</button>
          </span>
        </div>
      )}

      {/* 未连接时的选项 */}
      {!running && (
        <div className="tn-opts">
          <div className="field">
            <label>预授权 key（可选 —— <strong>留空即用 GitHub 登录入网</strong>,推荐;key 仅给无人值守的机器用）</label>
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
