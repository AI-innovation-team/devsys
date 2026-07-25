import { useEffect, useRef, useState } from "react";

import { data } from "../data";
import type { GhDeviceStart } from "../data";

// 与 AuthGate 一致的星座网背景坐标。
const NET_N = [[50, 18], [69, 24], [78, 41], [74, 60], [60, 72], [42, 74], [27, 63], [22, 44], [30, 27], [44, 36], [58, 44], [50, 56], [86, 33], [16, 55], [64, 12], [38, 88]];
const NET_E = [[0, 1], [1, 2], [2, 3], [3, 4], [4, 5], [5, 6], [6, 7], [7, 8], [8, 0], [9, 10], [10, 11], [9, 0], [11, 4], [2, 12], [7, 13], [0, 14], [5, 15]];

// GitHub 登录门（device flow）：登录 = GitHub 身份，本地不再验密码。
// 保险库已由设备密钥在后台自动解锁，这里只管「你是谁 + 属于哪个组织」。
//   configured=false → 引导先注册 OAuth App；始终给「先进本地」逃生口，不被卡死。
export function GithubGate({ onDone, onSkip }: { onDone: () => void; onSkip: () => void }) {
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [phase, setPhase] = useState<"idle" | "waiting" | "org">("idle");
  const [dev, setDev] = useState<GhDeviceStart | null>(null);
  const [orgs, setOrgs] = useState<string[]>([]);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const timer = useRef<number | null>(null);

  useEffect(() => {
    data.ghState().then((s) => {
      setConfigured(s.configured);
      if (s.logged_in && s.org) onDone(); // 已登录且选好组织 → 直接进
    }).catch(() => setConfigured(false));
    return () => { if (timer.current) window.clearTimeout(timer.current); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const copy = (t: string) => navigator.clipboard?.writeText?.(t).catch(() => {});

  const finish = (orgList: string[]) => {
    // 单个组织后端已自动选中；多个（或未选）→ 让用户挑，也可跳过。
    data.ghState().then((s) => {
      if (s.org) { onDone(); return; }
      if (orgList.length) { setOrgs(orgList); setPhase("org"); }
      else onDone(); // 没有 org 也放行：本地 SSH 不依赖组织
    }).catch(() => onDone());
  };

  const poll = (deviceCode: string, intervalMs: number) => {
    timer.current = window.setTimeout(async () => {
      try {
        const r = await data.ghDevicePoll(deviceCode);
        if (r.status === "ok") { setBusy(false); finish(r.orgs); return; }
        if (r.status === "error") { setBusy(false); setPhase("idle"); setErr(r.error || "授权失败，请重试"); return; }
        const next = r.status === "slow_down" ? intervalMs + 5000 : intervalMs;
        poll(deviceCode, next);
      } catch (e) {
        setBusy(false); setPhase("idle");
        setErr(e instanceof Error ? e.message : String(e));
      }
    }, intervalMs);
  };

  const start = async () => {
    setBusy(true); setErr("");
    try {
      const d = await data.ghDeviceStart();
      setDev(d);
      setPhase("waiting");
      // 自动弹浏览器到「码已预填」的直达链接 —— 用户只需在浏览器点 Authorize。
      const url = d.verification_uri_complete || d.verification_uri;
      data.openUrl(url).catch(() => {}); // 没弹出也无妨，UI 给手动兜底
      copy(d.user_code);                 // 顺手把码放进剪贴板（手动兜底用）
      poll(d.device_code, Math.max(1, d.interval) * 1000);
    } catch (e) {
      setBusy(false);
      setErr(e instanceof Error ? e.message : String(e));
    }
  };

  // 重新弹一次浏览器（用户误关了标签页）。
  const reopen = () => { if (dev) data.openUrl(dev.verification_uri_complete || dev.verification_uri).catch(() => {}); };

  const pickOrg = async (org: string) => {
    setBusy(true);
    try { await data.ghSetOrg(org); onDone(); }
    catch (e) { setBusy(false); setErr(e instanceof Error ? e.message : String(e)); }
  };

  return (
    <div className="gate">
      <svg className="gate-net" viewBox="0 0 100 100" preserveAspectRatio="xMidYMid slice" aria-hidden="true">
        <g className="gate-netg">
          {NET_E.map(([a, b], i) => (
            <line key={"e" + i} className="gate-edge" x1={NET_N[a][0]} y1={NET_N[a][1]} x2={NET_N[b][0]} y2={NET_N[b][1]} style={{ animationDelay: (i * 0.7).toFixed(2) + "s" }} />
          ))}
          {NET_N.map(([x, y], i) => (
            <circle key={"n" + i} className={"gate-node" + (i % 3 === 0 ? " dim" : "")} cx={x} cy={y} r={i % 4 === 0 ? 0.7 : 0.48} style={{ animationDelay: (i * 0.53).toFixed(2) + "s" }} />
          ))}
        </g>
      </svg>

      <div className="gate-inner">
        <span className="gate-tile">
          <svg className="gate-cube" width="54" height="54" viewBox="0 0 24 24" style={{ overflow: "visible" }} aria-label="AIT.dev">
            <path d="M12 2.6 20.5 7.3 12 12 3.5 7.3Z" fill="var(--accent)" className="gate-face-top" />
            <path d="M3.5 7.9 11.4 12.3 11.4 21.4 3.5 17Z" fill="currentColor" fillOpacity="0.9" />
            <path d="M20.5 7.9 12.6 12.3 12.6 21.4 20.5 17Z" fill="currentColor" fillOpacity="0.55" />
          </svg>
        </span>
        <div className="gate-name">AIT.dev</div>
        <div className="gate-sub">用 GitHub 登录 · 身份即组织成员</div>

        <div className="gate-stack">
          {err && (
            <div className="gate-err">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z" />
                <line x1="12" y1="9" x2="12" y2="13" /><line x1="12" y1="17" x2="12.01" y2="17" />
              </svg>
              <span>{err}</span>
            </div>
          )}

          {/* 未配 OAuth App：给出注册指引（device flow 需要一个公开 client_id）。 */}
          {configured === false && phase === "idle" && (
            <div className="gate-hint">
              还没配 GitHub OAuth App。去 <b>GitHub → Settings → Developer settings → OAuth Apps</b> 建一个，
              勾选 <b>Enable Device Flow</b>，把 <b>Client ID</b> 写进环境变量 <code>DEVSYS_GH_CLIENT_ID</code>
              或 app 配置目录的 <code>gh-client-id</code> 文件，重开即可登录。
            </div>
          )}

          {phase === "idle" && (
            <button className="gate-cta" disabled={busy || configured === false} onClick={start}>
              <GhMark /> 用 GitHub 登录
            </button>
          )}

          {/* 等待授权：浏览器已自动弹出（码预填），用户只需点 Authorize。下方是手动兜底。 */}
          {phase === "waiting" && dev && (
            <div className="gate-dev">
              <p className="gate-dev-wait"><span className="spin" /> 已打开浏览器，请点 <b>Authorize</b> 完成授权…</p>
              <button className="gate-mini gate-reopen" onClick={reopen}>没弹出？重新打开浏览器</button>
              <p className="gate-dev-fallback">或手动打开 <b>{dev.verification_uri}</b> 输入下面这个码（已复制到剪贴板）：</p>
              <div className="gate-code" onClick={() => copy(dev.user_code)} title="点击复制">{dev.user_code}</div>
            </div>
          )}

          {/* 选组织（多个 org 时）。 */}
          {phase === "org" && (
            <div className="gate-orgs">
              <p className="gate-dev-step">选择你的团队组织：</p>
              {orgs.map((o) => (
                <button key={o} className="gate-org" disabled={busy} onClick={() => pickOrg(o)}>{o}</button>
              ))}
              <button className="gate-link" onClick={onDone}>暂不选，先进入</button>
            </div>
          )}
        </div>

        <button className="gate-link" onClick={onSkip}>先进入本地（GitHub 稍后在设置里连）</button>
        <p className="gate-foot">凭据保险库已由本机设备密钥自动解锁 · 不再需要本地密码</p>
      </div>
    </div>
  );
}

function GhMark() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" style={{ marginRight: 8, verticalAlign: "-2px" }}>
      <path d="M12 2C6.48 2 2 6.58 2 12.25c0 4.53 2.87 8.37 6.84 9.73.5.1.68-.22.68-.49 0-.24-.01-.87-.01-1.71-2.78.62-3.37-1.37-3.37-1.37-.45-1.18-1.11-1.5-1.11-1.5-.91-.64.07-.62.07-.62 1 .07 1.53 1.06 1.53 1.06.89 1.56 2.34 1.11 2.91.85.09-.66.35-1.11.63-1.37-2.22-.26-4.56-1.14-4.56-5.07 0-1.12.39-2.03 1.03-2.75-.1-.26-.45-1.3.1-2.7 0 0 .84-.28 2.75 1.05a9.36 9.36 0 0 1 5 0c1.91-1.33 2.75-1.05 2.75-1.05.55 1.4.2 2.44.1 2.7.64.72 1.03 1.63 1.03 2.75 0 3.94-2.34 4.81-4.57 5.06.36.32.68.94.68 1.9 0 1.37-.01 2.48-.01 2.82 0 .27.18.6.69.49A10.02 10.02 0 0 0 22 12.25C22 6.58 17.52 2 12 2Z" />
    </svg>
  );
}
