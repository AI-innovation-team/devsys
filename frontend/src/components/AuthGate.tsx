import { useState } from "react";

import { data } from "../data";

// 与网页登录页（deploy/login/sign_in.html）一致的星座网坐标。
const NET_N = [[50, 18], [69, 24], [78, 41], [74, 60], [60, 72], [42, 74], [27, 63], [22, 44], [30, 27], [44, 36], [58, 44], [50, 56], [86, 33], [16, 55], [64, 12], [38, 88]];
const NET_E = [[0, 1], [1, 2], [2, 3], [3, 4], [4, 5], [5, 6], [6, 7], [7, 8], [8, 0], [9, 10], [10, 11], [9, 0], [11, 4], [2, 12], [7, 13], [0, 14], [5, 15]];

// 登录门：一个密码解锁本地保险库（密码 = 凭据钥匙）。没有「本地/远程」之分 ——
// 只有节点，团队是登录后 app 内的一等入口，不在登录门分叉。
// exists = 本地保险库已建（决定是「登录」还是「创建密码」）。
// onReset = 重置后回到「创建密码」态（密码不可找回，重置是唯一出路）。
export function AuthGate({ exists, onDone, onReset }: { exists: boolean; onDone: () => void; onReset: () => void }) {
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirmReset, setConfirmReset] = useState(false);

  const submit = async () => {
    if (!pw) { setErr("请输入密码"); return; }
    if (!exists) {
      if (pw.length < 6) { setErr("密码至少 6 位"); return; }
      if (pw !== pw2) { setErr("两次密码不一致"); return; }
    }
    setBusy(true);
    setErr("");
    try {
      await data.vaultUnlock(pw); // 首次=创建，之后=解锁
      onDone();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  const doReset = async () => {
    setBusy(true);
    setErr("");
    try {
      await data.vaultReset();
      setPw(""); setPw2(""); setConfirmReset(false);
      onReset(); // → exists=false，回到「创建密码」
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const cta = exists ? "登录" : "创建密码";
  const foot = exists ? "本地加密 · 密码即凭据钥匙" : "密码用于加密你的凭据，无法找回";

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
        <div className="gate-sub">DevSys of NeuroAI Innovation Team</div>

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
          <input className="gate-fld" type="password" autoFocus value={pw} onChange={(e) => setPw(e.target.value)} placeholder="密码" autoComplete={exists ? "current-password" : "new-password"} onKeyDown={(e) => e.key === "Enter" && submit()} />
          {!exists && (
            <input className="gate-fld" type="password" value={pw2} onChange={(e) => setPw2(e.target.value)} placeholder="确认密码" autoComplete="new-password" onKeyDown={(e) => e.key === "Enter" && submit()} />
          )}
          <button className="gate-cta" disabled={busy} onClick={submit}>{cta}</button>
        </div>

        {/* 密码不可找回（Argon2 派生密钥加密快照，明文不落盘），重置是唯一出路 —— 代价必须说清。 */}
        {exists && (confirmReset ? (
          <div className="gate-reset">
            <p className="gate-reset-warn">
              重置将<strong>永久销毁</strong>保险库里的所有凭据（密码、私钥），<strong>无法恢复</strong>。
              你的服务器列表会保留，之后需要重新录入凭据。
            </p>
            <div className="gate-reset-row">
              <button className="gate-reset-go" disabled={busy} onClick={doReset}>{busy ? "重置中…" : "确认重置"}</button>
              <button className="gate-reset-no" disabled={busy} onClick={() => setConfirmReset(false)}>取消</button>
            </div>
          </div>
        ) : (
          <button className="gate-link" onClick={() => { setConfirmReset(true); setErr(""); }}>忘记密码？</button>
        ))}

        <p className="gate-foot">{foot}</p>
      </div>
    </div>
  );
}
