import { useCallback, useEffect, useState } from "react";

import { AdminSrv, AdminUser, api, AuditLine, Me } from "../api";
import { Icon } from "../icons";

type Tab = "users" | "servers" | "logs";
const TABS: { k: Tab; label: string; icon: string }[] = [
  { k: "users", label: "用户", icon: "user" },
  { k: "servers", label: "服务器", icon: "server" },
  { k: "logs", label: "日志", icon: "logs" },
];

// 管理员界面（仅 config.yaml oauth.admins 可见）。先点 tab → 再看/编辑。
export function Admin({ me }: { me: Me | null }) {
  const [tab, setTab] = useState<Tab>("users");
  return (
    <div className="wrap">
      <header className="page-head">
        <h1>管理</h1>
        <p style={{ margin: "4px 0 0", color: "var(--text-faint)", fontSize: 14 }}>管理员：{me?.user}</p>
      </header>
      <nav className="admin-tabs">
        {TABS.map((t) => (
          <button key={t.k} className={tab === t.k ? "on" : ""} onClick={() => setTab(t.k)}>
            <Icon name={t.icon} />{t.label}
          </button>
        ))}
      </nav>
      {tab === "users" && <UsersTab />}
      {tab === "servers" && <ServersTab />}
      {tab === "logs" && <LogsTab />}
    </div>
  );
}

const fmtTime = (ts: number) => new Date(ts * 1000).toLocaleString("zh-CN", { hour12: false });

// ══ 用户 ══════════════════════════════════════════════════════
function UsersTab() {
  const [users, setUsers] = useState<AdminUser[] | null>(null);
  const load = useCallback(async () => {
    try { setUsers((await api.admin.users()).users); } catch { setUsers([]); }
  }, []);
  useEffect(() => { load(); }, [load]);
  const gh = (users || []).filter((u) => u.kind === "github");
  const em = (users || []).filter((u) => u.kind === "email");
  return (
    <section className="set-sec">
      <GithubGroup users={gh} loading={users === null} reload={load} />
      <EmailGroup users={em} loading={users === null} reload={load} />
    </section>
  );
}

// GitHub 用户 = oauth2 白名单：增删即改白名单（重启认证）。
function GithubGroup({ users, loading, reload }: { users: AdminUser[]; loading: boolean; reload: () => Promise<void> | void }) {
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const list = users.map((u) => u.user);

  const commit = async (next: string[]) => {
    setBusy(true); setNote("保存中…（重启认证约几秒）");
    try { await api.admin.setWhitelist(next); await reload(); setNote(""); setAdding(false); setName(""); }
    catch (e) { setNote("失败：" + String(e)); }
    setBusy(false);
  };
  const add = () => { const u = name.trim(); if (!u || list.includes(u)) { setName(""); return; } commit([...list, u]); };
  const del = (u: string) => { if (confirm(`移除 GitHub 用户 ${u}？将无法登录。`)) commit(list.filter((x) => x !== u)); };

  return (
    <>
      <div className="set-h"><h2>GitHub 用户</h2></div>
      {loading ? <p className="save-note">加载中…</p> : (
        <div className="ws-list">
          {users.length === 0 && <div className="ws-empty">暂无 GitHub 用户</div>}
          {users.map((u) => (
            <div className="ws-row" key={u.user}>
              <div className="ws-info">
                <span className="ws-dot on" />
                <span className="ws-name">{u.user}</span>
                {u.is_admin && <span className="badge accent"><Icon name="shield" />管理员</span>}
                {u.servers.length > 0 && <span className="badge ok">{u.servers.length} 台凭据</span>}
              </div>
              <div className="ws-acts">
                <button className="btn subtle sm" disabled={busy || u.is_admin} title={u.is_admin ? "不能移除管理员" : "移除"} onClick={() => del(u.user)}><Icon name="trash" /></button>
              </div>
            </div>
          ))}
        </div>
      )}
      {adding ? (
        <div className="cred-foot" style={{ marginTop: 10 }}>
          <div className="inp" style={{ flex: 1, maxWidth: 340 }}><Icon name="user" /><input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="GitHub 用户名" autoComplete="off" disabled={busy} onKeyDown={(e) => e.key === "Enter" && add()} /></div>
          <button className="btn primary sm" disabled={busy} onClick={add}><Icon name="plus" />添加</button>
          <button className="btn subtle sm" disabled={busy} onClick={() => { setAdding(false); setName(""); }}>取消</button>
          <span className="save-note">{note}</span>
        </div>
      ) : (
        <button className="add-row" onClick={() => setAdding(true)}><Icon name="plus" />添加 GitHub 用户</button>
      )}
    </>
  );
}

function EmailGroup({ users, loading, reload }: { users: AdminUser[]; loading: boolean; reload: () => Promise<void> | void }) {
  const [adding, setAdding] = useState(false);
  return (
    <>
      <div className="set-h" style={{ marginTop: 28 }}><h2>账密用户</h2></div>
      {loading ? <p className="save-note">加载中…</p> : (
        <div className="cards">
          {users.length === 0 && <div className="ws-empty">暂无账密用户</div>}
          {users.map((u) => <EmailCard key={u.user} u={u} reload={reload} />)}
        </div>
      )}
      {adding
        ? <AddEmailCard onDone={() => { setAdding(false); reload(); }} onCancel={() => setAdding(false)} />
        : <button className="add-row" onClick={() => setAdding(true)}><Icon name="plus" />添加账密用户</button>}
    </>
  );
}

function EmailCard({ u, reload }: { u: AdminUser; reload: () => Promise<void> | void }) {
  const [open, setOpen] = useState(false);
  const [pw, setPw] = useState("");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const reset = async () => {
    if (pw.length < 8) { setNote("密码至少 8 位"); return; }
    setBusy(true); setNote("重置中…（重启认证约几秒）");
    try { await api.admin.addEmail(u.user, pw); setNote("已重置 ✓"); setPw(""); } catch (e) { setNote("失败：" + String(e)); }
    setBusy(false);
  };
  const del = async () => {
    if (!confirm(`删除账密用户 ${u.user}？该账号将无法登录。`)) return;
    setBusy(true); setNote("删除中…");
    try { await api.admin.delEmail(u.user); await reload(); } catch (e) { setNote("失败：" + String(e)); setBusy(false); }
  };
  return (
    <article className={"card" + (open ? " open" : "")}>
      <button className="cfg-head tog" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <div className="srv-title">
          <span className={"srv-dot" + (u.whitelisted ? " ok" : "")} />
          <span className="srv-name">{u.user}</span>
          {u.servers.length > 0 && <span className="badge ok">{u.servers.length} 台凭据</span>}
        </div>
        <div className="tog-r"><Icon name="chevron" className="chev" /></div>
      </button>
      {open && (
        <div className="cfg-body">
          <div className="row2">
            <div className="field"><label>重置密码（≥8 位）</label>
              <div className="inp"><Icon name="lock" /><input type="text" value={pw} onChange={(e) => setPw(e.target.value)} placeholder="输入新密码" autoComplete="off" /></div>
            </div>
          </div>
          <div className="cred-foot">
            <button className="btn primary sm" disabled={busy} onClick={reset}><Icon name="save" />重置密码</button>
            <button className="btn subtle sm" disabled={busy} onClick={del}><Icon name="trash" />删除用户</button>
            <span className="save-note">{note}　<span style={{ color: "var(--text-faint)" }}>操作会重启认证（已登录不受影响）</span></span>
          </div>
        </div>
      )}
    </article>
  );
}

function AddEmailCard({ onDone, onCancel }: { onDone: () => void; onCancel: () => void }) {
  const [email, setEmail] = useState("");
  const [pw, setPw] = useState("");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const add = async () => {
    if (!email.includes("@")) { setNote("请输入有效邮箱"); return; }
    if (pw.length < 8) { setNote("密码至少 8 位"); return; }
    setBusy(true); setNote("保存中…（重启认证约几秒）");
    try { await api.admin.addEmail(email, pw); onDone(); } catch (e) { setNote("失败：" + String(e)); setBusy(false); }
  };
  return (
    <div className="card open" style={{ marginTop: 10 }}><div className="cfg-body">
      <div className="row2">
        <div className="field"><label>邮箱</label>
          <div className="inp"><Icon name="user" /><input autoFocus value={email} onChange={(e) => setEmail(e.target.value)} placeholder="user@example.com" autoComplete="off" /></div>
        </div>
        <div className="field"><label>初始密码（≥8 位）</label>
          <div className="inp"><Icon name="lock" /><input type="text" value={pw} onChange={(e) => setPw(e.target.value)} placeholder="至少 8 位" autoComplete="off" /></div>
        </div>
      </div>
      <div className="cred-foot">
        <button className="btn primary sm" disabled={busy} onClick={add}><Icon name="plus" />添加</button>
        <button className="btn subtle sm" disabled={busy} onClick={onCancel}>取消</button>
        <span className="save-note">{note}</span>
      </div>
    </div></div>
  );
}

// ══ 服务器 ════════════════════════════════════════════════════
function ServersTab() {
  const [rows, setRows] = useState<AdminSrv[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const load = useCallback(async () => {
    try { setRows((await api.admin.servers()).servers); } catch { setRows([]); }
  }, []);
  useEffect(() => { load(); }, [load]);

  const set = (i: number, k: keyof AdminSrv, v: string) =>
    setRows((r) => r!.map((row, j) => (j === i ? { ...row, [k]: k === "port" ? Number(v) || 0 : v } : row)));
  const addRow = () => setRows((r) => [...(r || []), { name: "", host: "", port: 22 }]);
  const delRow = (i: number) => setRows((r) => r!.filter((_, j) => j !== i));
  const save = async () => {
    setBusy(true); setNote("保存中…");
    try { const res = await api.admin.saveServers(rows!); setRows(res.servers); setNote("已保存 ✓（即时生效）"); }
    catch (e) { setNote("失败：" + String(e)); }
    setBusy(false);
  };

  return (
    <section className="set-sec">
      <div className="set-h"><h2>服务器</h2></div>
      {rows === null ? <p className="save-note">加载中…</p> : (
        <div className="cards">
          {rows.length === 0 && <div className="ws-empty">还没有服务器</div>}
          {rows.map((s, i) => <ServerCard key={i} s={s} set={(k, v) => set(i, k, v)} del={() => delRow(i)} />)}
        </div>
      )}
      <button className="add-row" onClick={addRow}><Icon name="plus" />添加一台服务器</button>
      <div className="cred-foot" style={{ marginTop: 16 }}>
        <button className="btn primary sm" disabled={busy || rows === null} onClick={save}><Icon name="save" />保存全部</button>
        <span className="save-note">{note}</span>
      </div>
    </section>
  );
}

function ServerCard({ s, set, del }: { s: AdminSrv; set: (k: keyof AdminSrv, v: string) => void; del: () => void }) {
  const [open, setOpen] = useState(!s.name);  // 新增的空卡默认展开
  return (
    <article className={"card" + (open ? " open" : "")}>
      <button className="cfg-head tog" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <div className="srv-title">
          <span className="srv-dot ok" />
          <span className="srv-name">{s.name || "（新服务器）"}</span>
          {s.host && <span className="badge">{s.host}:{s.port}</span>}
          {s.jump && <span className="badge">via {s.jump}</span>}
        </div>
        <div className="tog-r"><Icon name="chevron" className="chev" /></div>
      </button>
      {open && (
        <div className="cfg-body">
          <div className="row2">
            <div className="field"><label>名称</label><div className="inp"><Icon name="server" /><input value={s.name} onChange={(e) => set("name", e.target.value)} placeholder="turing" /></div></div>
            <div className="field"><label>主机 IP</label><div className="inp"><Icon name="network" /><input value={s.host} onChange={(e) => set("host", e.target.value)} placeholder="172.16.x.x" /></div></div>
          </div>
          <div className="row2">
            <div className="field"><label>SSH 端口</label><div className="inp"><input value={s.port} onChange={(e) => set("port", e.target.value)} /></div></div>
            <div className="field"><label>跳板（可选，填另一台的名称）</label><div className="inp"><input value={s.jump || ""} onChange={(e) => set("jump", e.target.value)} placeholder="留空=直连" /></div></div>
          </div>
          <div className="cred-foot">
            <button className="btn subtle sm" onClick={del}><Icon name="trash" />删除这台</button>
            <span className="save-note" style={{ color: "var(--text-faint)" }}>改完记得点下方「保存全部」</span>
          </div>
        </div>
      )}
    </article>
  );
}

// ══ 日志 ══════════════════════════════════════════════════════
const SRCS = [{ k: "audit", label: "操作/会话审计" }, { k: "oauth2", label: "登录认证" }, { k: "portal", label: "门户服务" }];
const ACT_LABEL: Record<string, string> = {
  add_email_user: "添加账密用户", del_email_user: "删除账密用户", save_servers: "保存服务器",
  set_whitelist: "改 GitHub 白名单", ssh: "SSH 连接", vscode: "打开 VSCode", ws_new: "新建工作区",
};
function LogsTab() {
  const [src, setSrc] = useState("audit");
  const [lines, setLines] = useState<AuditLine[] | null>(null);
  const [text, setText] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const load = useCallback(async (s: string) => {
    setLoading(true); setLines(null); setText(null);
    try { const r = await api.admin.logs(s, 200); setLines(r.lines || null); setText(r.text ?? null); }
    catch (e) { setText("读取失败：" + String(e)); }
    setLoading(false);
  }, []);
  useEffect(() => { load(src); }, [src, load]);

  return (
    <section className="set-sec">
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 12 }}>
        <div className="seg">
          {SRCS.map((s) => <button key={s.k} className={src === s.k ? "on" : ""} onClick={() => setSrc(s.k)}>{s.label}</button>)}
        </div>
        <button className="btn subtle sm" onClick={() => load(src)} title="刷新" style={{ marginLeft: "auto" }}><Icon name="refresh" /></button>
      </div>
      {loading && <p className="save-note">加载中…</p>}
      {lines && (
        <div className="ws-list">
          {lines.length === 0 && <div className="ws-empty">暂无记录</div>}
          {lines.map((l, i) => (
            <div className="ws-row" key={i}>
              <div className="ws-info">
                <span className="ws-name">{ACT_LABEL[l.action] || l.action}</span>
                <span className="badge"><Icon name="user" />{l.actor}</span>
                {"server" in l && <span className="badge accent"><Icon name="server" />{String(l.server)}</span>}
                {"target" in l && <span className="badge">{String(l.target)}</span>}
              </div>
              <span className="save-note" style={{ flexShrink: 0 }}>{fmtTime(l.ts)}</span>
            </div>
          ))}
        </div>
      )}
      {text != null && (
        <div className="logbox">
          {text.split("\n").filter((x) => x.trim()).map((ln, i) => {
            const m = ln.match(/^(\S+)\s+\S+\s+\S+\[\d+\]:\s*(.*)$/);
            const time = m ? m[1].replace(/\+\d{4}$/, "").replace("T", " ") : "";
            const msg = m ? m[2] : ln;
            return (
              <div className="logline" key={i}>
                {time && <span className="t">{time}</span>}
                <span className="m">{msg}</span>
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
