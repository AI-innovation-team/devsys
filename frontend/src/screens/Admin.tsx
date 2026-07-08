import { useCallback, useEffect, useState } from "react";

import { AdminSrv, AdminUser, api, AuditLine, Me } from "../api";
import { Icon } from "../icons";

// 管理员界面（仅 config.yaml oauth.admins 可见）。
export function Admin({ me }: { me: Me | null }) {
  return (
    <div className="wrap">
      <header className="page-head">
        <h1>管理</h1>
        <p style={{ margin: "4px 0 0", color: "var(--text-faint)", fontSize: 14 }}>
          服务器 · 用户 · 权限 · 日志　（管理员：{me?.user}）
        </p>
      </header>
      <UsersSection />
      <EmailSection />
      <ServersSection />
      <WhitelistSection />
      <LogsSection />
    </div>
  );
}

const fmtTime = (ts: number) => new Date(ts * 1000).toLocaleString("zh-CN", { hour12: false });

// ── 用户总览（只读）────────────────────────────────────────────
function UsersSection() {
  const [users, setUsers] = useState<AdminUser[] | null>(null);
  const load = useCallback(async () => {
    try { setUsers((await api.admin.users()).users); } catch { setUsers([]); }
  }, []);
  useEffect(() => { load(); }, [load]);
  return (
    <section className="set-sec">
      <div className="set-h"><h2>用户总览</h2></div>
      {users === null ? <p className="save-note">加载中…</p> : (
        <div className="ws-list">
          {users.length === 0 && <div className="ws-empty">暂无用户</div>}
          {users.map((u) => (
            <div className="ws-row" key={u.user}>
              <div className="ws-info">
                <span className={"ws-dot" + (u.whitelisted ? " on" : "")} />
                <span className="ws-name">{u.user}</span>
                <span className="badge">{u.kind === "email" ? "邮箱" : "GitHub"}</span>
                {u.is_admin && <span className="badge accent"><Icon name="shield" />管理员</span>}
                {u.servers.length > 0 && <span className="badge ok">{u.servers.length} 台凭据</span>}
              </div>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

// ── 邮箱用户管理（增删，会重启 oauth2）───────────────────────────
function EmailSection() {
  const [users, setUsers] = useState<AdminUser[] | null>(null);
  const [email, setEmail] = useState("");
  const [pw, setPw] = useState("");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const load = useCallback(async () => {
    try { setUsers((await api.admin.users()).users.filter((u) => u.kind === "email")); } catch { setUsers([]); }
  }, []);
  useEffect(() => { load(); }, [load]);

  const add = async () => {
    if (!email.includes("@")) { setNote("请输入有效邮箱"); return; }
    if (pw.length < 8) { setNote("密码至少 8 位"); return; }
    setBusy(true); setNote("保存中…（重启认证约几秒）");
    try { await api.admin.addEmail(email, pw); setNote("已添加 ✓"); setEmail(""); setPw(""); await load(); }
    catch (e) { setNote("失败：" + String(e)); }
    setBusy(false);
  };
  const del = async (m: string) => {
    if (!confirm(`删除邮箱用户 ${m}？该账号将无法登录。`)) return;
    setNote("删除中…");
    try { await api.admin.delEmail(m); setNote("已删除 ✓"); await load(); }
    catch (e) { setNote("失败：" + String(e)); }
  };

  return (
    <section className="set-sec">
      <div className="set-h"><h2>邮箱用户</h2></div>
      <div className="card"><div className="cfg-body">
        <div className="row2">
          <div className="field"><label>邮箱</label>
            <div className="inp"><Icon name="user" /><input value={email} onChange={(e) => setEmail(e.target.value)} placeholder="user@example.com" autoComplete="off" /></div>
          </div>
          <div className="field"><label>初始密码（≥8 位）</label>
            <div className="inp"><Icon name="lock" /><input type="text" value={pw} onChange={(e) => setPw(e.target.value)} placeholder="至少 8 位" autoComplete="off" /></div>
          </div>
        </div>
        <div className="cred-foot">
          <button className="btn primary sm" disabled={busy} onClick={add}><Icon name="plus" />添加 / 重置</button>
          <span className="save-note">{note}　<span style={{ color: "var(--text-faint)" }}>增删会重启认证（已登录不受影响）</span></span>
        </div>
      </div></div>
      <div className="ws-list" style={{ marginTop: 12 }}>
        {users?.length === 0 && <div className="ws-empty">还没有邮箱用户</div>}
        {users?.map((u) => (
          <div className="ws-row" key={u.user}>
            <div className="ws-info"><span className="ws-dot on" /><span className="ws-name">{u.user}</span></div>
            <div className="ws-acts"><button className="btn subtle sm" onClick={() => del(u.user)}><Icon name="trash" /></button></div>
          </div>
        ))}
      </div>
    </section>
  );
}

// ── 服务器增删改（运行时真源，即时生效）─────────────────────────
function ServersSection() {
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
      <div className="card"><div className="cfg-body">
        {rows === null ? <p className="save-note">加载中…</p> : rows.map((s, i) => (
          <div className="row2" key={i} style={{ gridTemplateColumns: "1fr 1.4fr .7fr 1fr auto", gap: 8, alignItems: "end", marginBottom: 8 }}>
            <div className="field"><label>名称</label><div className="inp"><input value={s.name} onChange={(e) => set(i, "name", e.target.value)} placeholder="turing" /></div></div>
            <div className="field"><label>主机</label><div className="inp"><input value={s.host} onChange={(e) => set(i, "host", e.target.value)} placeholder="172.16.x.x" /></div></div>
            <div className="field"><label>端口</label><div className="inp"><input value={s.port} onChange={(e) => set(i, "port", e.target.value)} /></div></div>
            <div className="field"><label>跳板（可选）</label><div className="inp"><input value={s.jump || ""} onChange={(e) => set(i, "jump", e.target.value)} placeholder="另一台的名称" /></div></div>
            <button className="btn subtle sm" onClick={() => delRow(i)} title="删除"><Icon name="trash" /></button>
          </div>
        ))}
        <div className="cred-foot">
          <button className="btn secondary sm" onClick={addRow}><Icon name="plus" />添加一台</button>
          <button className="btn primary sm" disabled={busy} onClick={save}><Icon name="save" />保存全部</button>
          <span className="save-note">{note}</span>
        </div>
      </div></div>
    </section>
  );
}

// ── GitHub 白名单（原地改 cfg + 重启）───────────────────────────
function WhitelistSection() {
  const [list, setList] = useState<string[] | null>(null);
  const [add, setAdd] = useState("");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const load = useCallback(async () => {
    try { setList((await api.admin.whitelist()).github_users); } catch { setList([]); }
  }, []);
  useEffect(() => { load(); }, [load]);

  const commit = async (next: string[]) => {
    setBusy(true); setNote("保存中…（重启认证约几秒）");
    try { const res = await api.admin.setWhitelist(next); setList(res.github_users); setNote("已保存 ✓"); }
    catch (e) { setNote("失败：" + String(e)); }
    setBusy(false);
  };
  const onAdd = () => {
    const u = add.trim();
    if (!u || list!.includes(u)) { setAdd(""); return; }
    commit([...list!, u]); setAdd("");
  };
  const onDel = (u: string) => { if (confirm(`从白名单移除 GitHub 用户 ${u}？`)) commit(list!.filter((x) => x !== u)); };

  return (
    <section className="set-sec">
      <div className="set-h"><h2>GitHub 白名单</h2></div>
      <div className="card"><div className="cfg-body">
        <div className="cred-foot" style={{ marginBottom: 8 }}>
          <div className="inp" style={{ flex: 1 }}><Icon name="user" /><input value={add} onChange={(e) => setAdd(e.target.value)} placeholder="GitHub 用户名" autoComplete="off" onKeyDown={(e) => e.key === "Enter" && onAdd()} /></div>
          <button className="btn primary sm" disabled={busy} onClick={onAdd}><Icon name="plus" />添加</button>
        </div>
        <div className="ws-list">
          {list === null ? <p className="save-note">加载中…</p> : list.length === 0 ? <div className="ws-empty">白名单为空</div> : list.map((u) => (
            <div className="ws-row" key={u}>
              <div className="ws-info"><span className="ws-dot on" /><span className="ws-name">{u}</span></div>
              <div className="ws-acts"><button className="btn subtle sm" onClick={() => onDel(u)}><Icon name="trash" /></button></div>
            </div>
          ))}
        </div>
        <span className="save-note">{note}　<span style={{ color: "var(--text-faint)" }}>增删会重启认证（已登录不受影响）</span></span>
      </div></div>
    </section>
  );
}

// ── 日志 ────────────────────────────────────────────────────────
const SRCS = [{ k: "audit", label: "操作/会话审计" }, { k: "oauth2", label: "登录认证" }, { k: "portal", label: "门户服务" }];
function LogsSection() {
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
      <div className="set-h"><h2>日志</h2></div>
      <div className="card"><div className="cfg-body">
        <div className="seg" style={{ marginBottom: 10 }}>
          {SRCS.map((s) => <button key={s.k} className={src === s.k ? "on" : ""} onClick={() => setSrc(s.k)}>{s.label}</button>)}
          <button onClick={() => load(src)} title="刷新" style={{ marginLeft: "auto" }}><Icon name="refresh" /></button>
        </div>
        {loading && <p className="save-note">加载中…</p>}
        {lines && (
          <div className="ws-list">
            {lines.length === 0 && <div className="ws-empty">暂无记录</div>}
            {lines.map((l, i) => (
              <div className="ws-row" key={i}>
                <div className="ws-info">
                  <span className="ws-name">{l.action}</span>
                  <span className="badge">{l.actor}</span>
                  {"target" in l && <span className="badge">{String(l.target)}</span>}
                  {"server" in l && <span className="badge accent">{String(l.server)}</span>}
                </div>
                <span className="save-note">{fmtTime(l.ts)}</span>
              </div>
            ))}
          </div>
        )}
        {text != null && (
          <pre style={{ margin: 0, maxHeight: 420, overflow: "auto", fontFamily: "var(--font-mono)", fontSize: 12, lineHeight: 1.5, color: "var(--text-body)", whiteSpace: "pre-wrap", wordBreak: "break-word" }}>{text}</pre>
        )}
      </div></div>
    </section>
  );
}
