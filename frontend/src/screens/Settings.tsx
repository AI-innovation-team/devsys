import { useState } from "react";

import { Theme } from "../App";
import { api, Me, Server } from "../api";
import { Icon } from "../icons";

interface Props {
  me: Me | null;
  reload: () => Promise<void> | void;
  theme: Theme;
  setTheme: (t: Theme) => void;
}

export function Settings({ me, reload, theme, setTheme }: Props) {
  const servers = me?.servers || [];
  return (
    <div className="wrap">
      <header className="page-head"><h1>设置</h1></header>

      {me?.email_login && <AccountSection email={me.user} />}

      <section className="set-sec">
        <div className="set-h"><h2>连接凭据</h2></div>
        <div className="cards">{servers.map((s) => <CredCard key={s.name} s={s} reload={reload} />)}</div>
      </section>

      <section className="set-sec">
        <div className="set-h"><h2>外观</h2></div>
        <div className="card"><div className="cfg-body"><div className="set-row">
          <span className="lbl2">主题</span>
          <div className="seg">
            <button className={theme === "light" ? "on" : ""} onClick={() => setTheme("light")}>浅色</button>
            <button className={theme === "dark" ? "on" : ""} onClick={() => setTheme("dark")}>深色</button>
          </div>
        </div></div></div>
      </section>
    </div>
  );
}

function CredCard({ s, reload }: { s: Server; reload: () => Promise<void> | void }) {
  const ready = !!(s.has_secret && s.username);
  const [username, setUsername] = useState(s.username || "");
  const [auth, setAuth] = useState<"password" | "key">(s.auth || "password");
  const [pw, setPw] = useState("");
  const [key, setKey] = useState("");
  const [note, setNote] = useState("");
  const [saving, setSaving] = useState(false);

  const save = async () => {
    setSaving(true);
    setNote("保存中…");
    const secret = auth === "key" ? key : pw;
    try {
      await api.saveSettings({ server: s.name, username, auth, ...(secret ? { secret } : {}) });
      setNote("已保存 ✓");
      setPw("");
      setKey("");
      await reload();
    } catch {
      setNote("保存失败");
    }
    setSaving(false);
  };

  return (
    <article className="card">
      <div className="cfg-head">
        <div className="srv-title">
          <span className={"srv-dot" + (ready ? " ok" : "")} />
          <span className="srv-name">{s.name}</span>
          <span className="badge">{s.host}:{s.port}</span>
          {s.jump && <span className="badge">via {s.jump}</span>}
        </div>
        <span className={"badge " + (ready ? "ok" : "warn")}>
          <Icon name={ready ? "check" : "alert"} />{ready ? "凭据就绪" : "未设置"}
        </span>
      </div>
      <div className="cfg-body">
        <div className="row2">
          <div className="field">
            <label>用户名 (userid)</label>
            <div className="inp"><Icon name="user" /><input value={username} onChange={(e) => setUsername(e.target.value)} placeholder="如 alice" autoComplete="off" /></div>
          </div>
          <div className="field">
            <label>认证方式</label>
            <div className="inp"><Icon name="key" /><select value={auth} onChange={(e) => setAuth(e.target.value as "password" | "key")}><option value="password">密码</option><option value="key">SSH 私钥</option></select></div>
          </div>
        </div>
        {auth === "password" ? (
          <div className="field">
            <label>密码</label>
            <div className="inp"><Icon name="lock" /><input type="password" value={pw} onChange={(e) => setPw(e.target.value)} placeholder={s.has_secret ? "已保存，留空则不改" : "输入密码"} /></div>
          </div>
        ) : (
          <div className="field">
            <label>SSH 私钥</label>
            <textarea className="ta" value={key} onChange={(e) => setKey(e.target.value)} placeholder={s.has_secret ? "已保存，留空则不改" : "-----BEGIN OPENSSH PRIVATE KEY-----"} />
          </div>
        )}
        <div className="cred-foot">
          <button className="btn primary sm" disabled={saving} onClick={save}><Icon name="save" />保存凭据</button>
          <span className="save-note">{note}</span>
        </div>
      </div>
    </article>
  );
}

function AccountSection({ email }: { email: string }) {
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const save = async () => {
    if (pw.length < 8) { setNote("密码至少 8 位"); return; }
    if (pw !== pw2) { setNote("两次输入不一致"); return; }
    setBusy(true);
    setNote("修改中…");
    try {
      await api.changePassword(pw);
      setNote("已修改 ✓（下次登录用新密码）");
      setPw("");
      setPw2("");
    } catch {
      setNote("修改失败");
    }
    setBusy(false);
  };
  return (
    <section className="set-sec">
      <div className="set-h"><h2>账户</h2></div>
      <div className="card">
        <div className="cfg-head">
          <div className="srv-title"><Icon name="user" /><span className="srv-name" style={{ fontSize: 16 }}>{email}</span></div>
          <span className="badge">邮箱登录</span>
        </div>
        <div className="cfg-body">
          <div className="row2">
            <div className="field"><label>新密码</label><div className="inp"><Icon name="lock" /><input type="password" value={pw} onChange={(e) => setPw(e.target.value)} placeholder="至少 8 位" autoComplete="new-password" /></div></div>
            <div className="field"><label>确认新密码</label><div className="inp"><Icon name="lock" /><input type="password" value={pw2} onChange={(e) => setPw2(e.target.value)} placeholder="再输一次" autoComplete="new-password" /></div></div>
          </div>
          <div className="cred-foot">
            <button className="btn primary sm" disabled={busy} onClick={save}><Icon name="save" />修改密码</button>
            <span className="save-note">{note}</span>
          </div>
        </div>
      </div>
    </section>
  );
}
