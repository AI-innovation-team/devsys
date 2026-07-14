import { useState } from "react";

import { Me, Server } from "../api";
import { data, supportsLocalTopology, type ServerInput, type SshHost } from "../data";
import { ImportModal } from "../components/ImportModal";
import { Icon } from "../icons";

type TKind = "direct" | "jump" | "tailnet";
const TRANSPORTS: { v: TKind; label: string; hint: string }[] = [
  { v: "direct", label: "直连", hint: "同一 LAN / VPN 内直达" },
  { v: "jump", label: "跳板", hint: "经另一台服务器 ProxyJump" },
  { v: "tailnet", label: "Tailnet", hint: "经 Tailscale 直达" },
];

// 按来源分组：mine 永远第一组，其后是各团队（team:<名>）。
// 呼应「万物皆节点、按来源合并」——一份列表里区分自持节点与团队共享节点。
type Group = { key: string; label: string; team: boolean; items: Server[] };
function groupBySource(servers: Server[]): Group[] {
  const by = new Map<string, Server[]>();
  for (const s of servers) {
    const src = s.source || "mine";
    (by.get(src) ?? by.set(src, []).get(src)!).push(s);
  }
  const groups: Group[] = [];
  if (by.has("mine")) groups.push({ key: "mine", label: "我的服务器", team: false, items: by.get("mine")! });
  for (const key of [...by.keys()].filter((k) => k !== "mine").sort()) {
    const name = key.startsWith("team:") ? key.slice(5) : key;
    groups.push({ key, label: `团队 · ${name}`, team: true, items: by.get(key)! });
  }
  return groups;
}

export function Servers({
  me,
  reload,
  goSettings,
  goTerminal,
}: {
  me: Me | null;
  reload: () => Promise<void> | void;
  goSettings: () => void;
  goTerminal: (name: string) => void;
}) {
  const servers = me?.servers || [];
  const local = supportsLocalTopology;
  const [editing, setEditing] = useState<Server | "new" | null>(null);
  const [imp, setImp] = useState<{ path: string; hosts: SshHost[] } | null>(null);
  const [impErr, setImpErr] = useState("");

  const loadConfig = async (path?: string) => {
    setImpErr("");
    try {
      const r = await data.readSshConfig(path);
      if (!r.hosts.length) { setImpErr(`${r.path} 里没有可导入的主机`); return; }
      setImp(r);
    } catch (e) {
      setImpErr(e instanceof Error ? e.message : String(e));
    }
  };

  const groups = groupBySource(servers);
  const multi = groups.length > 1; // 有团队源时才显示分组标题，纯本地时不加噪

  return (
    <div className="wrap">
      <header className="page-head"><h1>服务器</h1></header>

      {groups.map((g) => (
        <section key={g.key} className="srv-group">
          {multi && (
            <div className="srv-group-head">
              <span className="srv-group-name">{g.label}</span>
              {g.team && <span className="srv-group-tag">只读 · 团队共享</span>}
            </div>
          )}
          <div className="cards">
            {g.items.map((s) => (
              <LaunchCard
                key={s.name}
                s={s}
                local={local}
                readonly={g.team}
                goSettings={goSettings}
                goTerminal={goTerminal}
                onEdit={() => setEditing(s)}
                onDelete={async () => { await data.delServer(s.name); await reload(); }}
              />
            ))}
          </div>
        </section>
      ))}

      {local && (editing ? (
        <ServerForm
          initial={editing === "new" ? null : editing}
          servers={servers}
          onCancel={() => setEditing(null)}
          onSaved={async () => { setEditing(null); await reload(); }}
        />
      ) : (
        <>
          <button className="add-row" onClick={() => setEditing("new")}>
            <Icon name="plus" />添加服务器
          </button>
          <button className="import-btn" onClick={() => loadConfig()}>
            <Icon name="upload" />导入本机配置
          </button>
          {impErr && <div className="import-err">{impErr}</div>}
        </>
      ))}

      {imp && (
        <ImportModal
          path={imp.path}
          hosts={imp.hosts}
          existing={new Set(servers.map((s) => s.name))}
          onPick={async () => {
            const p = await data.pickSshConfigFile();
            if (p) await loadConfig(p);
          }}
          onCancel={() => setImp(null)}
          onImport={async (chosen) => {
            await data.importHosts(chosen);
            await reload();
            setImp(null);
          }}
        />
      )}
    </div>
  );
}

function LaunchCard({
  s,
  local,
  readonly,
  goSettings,
  goTerminal,
  onEdit,
  onDelete,
}: {
  s: Server;
  local: boolean;
  readonly: boolean; // 团队来源：拓扑只读（仍可设自己的凭据、SSH）
  goSettings: () => void;
  goTerminal: (name: string) => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const ready = !!(s.has_secret && s.username);
  const [confirming, setConfirming] = useState(false);
  return (
    <article className="card">
      <div className="card-head">
        <div>
          <div className="srv-title">
            <span className={"srv-dot" + (ready ? " ok" : "")} />
            <span className="srv-name">{s.name}</span>
            {s.transport === "tailnet" && <span className="badge">tailnet</span>}
            {s.jump && <span className="badge">via {s.jump}</span>}
          </div>
          <div className="srv-host"><Icon name="network" />{s.host}:{s.port}</div>
        </div>
        <div className="srv-actions">
          {ready ? (
            local ? (
              <button className="btn secondary" onClick={() => goTerminal(s.name)}><Icon name="terminal" />SSH</button>
            ) : (
              <>
                <a className="btn secondary" href={`/terminal/${encodeURIComponent(s.name)}`} target="_blank" rel="noreferrer"><Icon name="terminal" />SSH</a>
                <a className="btn primary" href={`/vscode/${encodeURIComponent(s.name)}`} target="_blank" rel="noreferrer"><Icon name="code" />VS Code</a>
              </>
            )
          ) : (
            <button className="btn subtle" onClick={goSettings}><Icon name="settings" />设置凭据</button>
          )}
          {local && !readonly && (
            confirming ? (
              <>
                <button className="btn secondary sm" onClick={onDelete}>确认删除</button>
                <button className="btn subtle sm" onClick={() => setConfirming(false)}>取消</button>
              </>
            ) : (
              <>
                <button className="btn subtle sm" title="编辑" onClick={onEdit}><Icon name="pencil" /></button>
                <button className="btn subtle sm" title="删除" onClick={() => setConfirming(true)}><Icon name="trash" /></button>
              </>
            )
          )}
        </div>
      </div>
    </article>
  );
}

function ServerForm({
  initial,
  servers,
  onCancel,
  onSaved,
}: {
  initial: Server | null;
  servers: Server[];
  onCancel: () => void;
  onSaved: () => void | Promise<void>;
}) {
  const isNew = !initial;
  const [name, setName] = useState(initial?.name || "");
  const [host, setHost] = useState(initial?.host || "");
  const [port, setPort] = useState(String(initial?.port ?? 22));
  const [transport, setTransport] = useState<TKind>(
    initial?.transport || (initial?.jump ? "jump" : "direct"),
  );
  const [jump, setJump] = useState(initial?.jump || "");
  const [note, setNote] = useState("");
  const [saving, setSaving] = useState(false);

  const others = servers.filter((x) => x.name !== name);

  const save = async () => {
    if (!name.trim() || !host.trim()) { setNote("名称与主机必填"); return; }
    const p = Number(port);
    if (!p || p < 1 || p > 65535) { setNote("端口无效"); return; }
    if (transport === "jump" && !jump) { setNote("跳板模式需选择跳板服务器"); return; }
    setSaving(true);
    setNote("保存中…");
    const input: ServerInput = {
      name: name.trim(),
      host: host.trim(),
      port: p,
      transport,
      jump: transport === "jump" ? jump : null,
      // 编辑时保留已有 username/auth（凭据在「设置」里管，勿被拓扑保存覆盖）。
      username: initial?.username ?? "",
      auth: initial?.auth ?? "password",
    };
    try {
      await data.upsertServer(input);
      await onSaved();
    } catch (e) {
      setNote("保存失败：" + (e as Error).message);
      setSaving(false);
    }
  };

  return (
    <article className="card open" style={{ marginTop: 12 }}>
      <div className="cfg-head">
        <div className="srv-title"><span className="srv-name">{isNew ? "添加服务器" : "编辑 " + initial!.name}</span></div>
      </div>
      <div className="cfg-body">
        <div className="row2">
          <div className="field">
            <label>名称</label>
            <div className="inp"><Icon name="server" /><input value={name} disabled={!isNew} onChange={(e) => setName(e.target.value)} placeholder="如 gpu-01" autoComplete="off" /></div>
          </div>
          <div className="field">
            <label>SSH 端口</label>
            <div className="inp"><Icon name="network" /><input value={port} onChange={(e) => setPort(e.target.value)} inputMode="numeric" placeholder="22" /></div>
          </div>
        </div>
        <div className="field">
          <label>主机地址</label>
          <div className="inp"><Icon name="network" /><input value={host} onChange={(e) => setHost(e.target.value)} placeholder="192.168.1.10 或 tailnet 名" autoComplete="off" /></div>
        </div>
        <div className="field">
          <label>连接方式</label>
          <div className="seg">
            {TRANSPORTS.map((t) => (
              <button key={t.v} className={transport === t.v ? "on" : ""} onClick={() => setTransport(t.v)} title={t.hint}>{t.label}</button>
            ))}
          </div>
        </div>
        {transport === "jump" && (
          <div className="field">
            <label>跳板服务器</label>
            <div className="inp"><Icon name="network" />
              <select value={jump} onChange={(e) => setJump(e.target.value)}>
                <option value="">选择…</option>
                {others.map((x) => <option key={x.name} value={x.name}>{x.name}（{x.host}）</option>)}
              </select>
            </div>
          </div>
        )}
        <div className="cred-foot">
          <button className="btn primary sm" disabled={saving} onClick={save}><Icon name="save" />{isNew ? "添加" : "保存"}</button>
          <button className="btn subtle sm" onClick={onCancel}>取消</button>
          <span className="save-note">{note}</span>
        </div>
      </div>
    </article>
  );
}
