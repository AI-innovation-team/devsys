import { useEffect, useState } from "react";

import { Me, Server } from "../api";
import { data, supportsLocalTopology, type ServerInput, type SshHost } from "../data";
import { ImportModal } from "../components/ImportModal";
import { ProvisionModal } from "../components/ProvisionModal";
import { SelfNodeModal } from "../components/SelfNodeModal";
import { Icon } from "../icons";

type TKind = "direct" | "jump" | "tailnet";
const TRANSPORTS: { v: TKind; label: string; hint: string }[] = [
  { v: "direct", label: "直连", hint: "同一 LAN / VPN 内直达" },
  { v: "jump", label: "跳板", hint: "经另一台服务器 ProxyJump" },
  { v: "tailnet", label: "Tailnet", hint: "经 Tailscale 直达" },
];

// 贡献机器时选的「开放档位」—— 机器主人决定给团队开多大权，也为此担责。
const TIERS: { v: number; label: string; hint: string }[] = [
  { v: 0, label: "档 0 · 纯跳板", hint: "只借道转发，队友在这台机上没有 shell" },
  { v: 1, label: "档 1 · 受限计算", hint: "每人独立账号、无 sudo，能跑计算（推荐）" },
  { v: 2, label: "档 2 · 完全信任", hint: "有 sudo —— 仅限核心成员" },
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
  teamPath,
  goTeam,
}: {
  me: Me | null;
  reload: () => Promise<void> | void;
  goSettings: () => void;
  goTerminal: (name: string) => void;
  teamPath: string; // 当前团队 team.yaml；空 = 还没连团队
  goTeam: () => void;
}) {
  const servers = me?.servers || [];
  const local = supportsLocalTopology;
  const [editing, setEditing] = useState<Server | "new" | null>(null);
  const [imp, setImp] = useState<{ path: string; hosts: SshHost[] } | null>(null);
  const [impErr, setImpErr] = useState("");
  // 正在下发授权的机器（打开 ProvisionModal）
  const [prov, setProv] = useState<{ server: string } | null>(null);
  const [addSelf, setAddSelf] = useState(false); // 把本机登记成节点

  // 团队角色（贡献时按角色开档）+ 我的成员名（写进我的 members/<我>.yaml）。
  const [roles, setRoles] = useState<string[]>(["core", "member", "guest"]);
  const myName = me?.user || "";
  useEffect(() => {
    if (!teamPath) return;
    data.readTeamView(teamPath)
      .then((v) => setRoles(Object.keys(v.roles)))
      .catch(() => {});
  }, [teamPath]);

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
                teamPath={teamPath}
                roles={roles}
                myName={myName}
                goSettings={goSettings}
                goTerminal={goTerminal}
                goTeam={goTeam}
                reload={reload}
                onEdit={() => setEditing(s)}
                onDelete={async () => { await data.delServer(s.name); await reload(); }}
                onProvision={() => setProv({ server: s.name })}
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
          <div className="row-btns">
            <button className="import-btn" onClick={() => loadConfig()}>
              <Icon name="upload" />导入本机配置
            </button>
            {/* 本机也是节点：可作算力贡献，也可作通往你内网的跳板 */}
            <button className="import-btn" onClick={() => setAddSelf(true)}>
              <Icon name="server" />添加本机
            </button>
          </div>
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

      {prov && (
        <ProvisionModal
          teamPath={teamPath}
          server={prov.server}
          onClose={() => setProv(null)}
        />
      )}

      {addSelf && (
        <SelfNodeModal
          existing={new Set(servers.map((s) => s.name))}
          onCancel={() => setAddSelf(false)}
          onAdded={async () => { await reload(); setAddSelf(false); }}
        />
      )}
    </div>
  );
}

const TIER_SHORT = ["借道", "受限", "信任"]; // 档 0/1/2 简称

function LaunchCard({
  s,
  local,
  readonly,
  teamPath,
  roles,
  myName,
  goSettings,
  goTerminal,
  goTeam,
  reload,
  onEdit,
  onDelete,
  onProvision,
}: {
  s: Server;
  local: boolean;
  readonly: boolean; // 团队来源：拓扑只读（仍可设自己的凭据、SSH）
  teamPath: string;
  roles: string[];
  myName: string;
  goSettings: () => void;
  goTerminal: (name: string) => void;
  goTeam: () => void;
  reload: () => Promise<void> | void;
  onEdit: () => void;
  onDelete: () => void;
  onProvision: () => void;
}) {
  const ready = !!(s.has_secret && s.username);
  const [confirming, setConfirming] = useState(false);
  const [sharing, setSharing] = useState(false); // 展开角色授权面板
  // grants：每个角色开的档位。默认 core=2 / member=1 / guest=0（缺省 1）。
  const defaultTier = (r: string) => (r === "core" ? 2 : r === "guest" ? 0 : 1);
  const [grants, setGrants] = useState<Record<string, number>>({});
  const [busy, setBusy] = useState(false);
  const [shareErr, setShareErr] = useState("");

  const shared = (s.shared_to?.length ?? 0) > 0;
  const sharedTeam = s.shared_to?.[0]?.replace(/^team:/, "") ?? "";

  const openShare = () => {
    // 初始化 grants（按角色默认）
    const g: Record<string, number> = {};
    for (const r of roles) g[r] = defaultTier(r);
    setGrants(g);
    setSharing(true);
    setShareErr("");
  };

  const doShare = async () => {
    setBusy(true); setShareErr("");
    try {
      await data.shareServer(teamPath, myName, s.name, grants);
      await reload();
      setSharing(false);
    } catch (e) {
      setShareErr(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); }
  };

  const doUnshare = async () => {
    setBusy(true); setShareErr("");
    try {
      await data.unshareServer(teamPath, myName, s.name);
      await reload();
    } catch (e) {
      setShareErr(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); }
  };

  return (
    <article className="card">
      <div className="card-head">
        <div>
          <div className="srv-title">
            <span className={"srv-dot" + (ready ? " ok" : "")} />
            <span className="srv-name">{s.name}</span>
            {s.transport === "tailnet" && <span className="badge">tailnet</span>}
            {s.jump && <span className="badge">via {s.jump}</span>}
            {shared && <span className="badge accent"><Icon name="users" />共享给 {sharedTeam}</span>}
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
                {/* 贡献：把这台机的拓扑给团队（凭据不出本机）。未连团队 → 引导去连。 */}
                <button
                  className="btn subtle sm"
                  title={shared ? "共享设置" : "共享给团队"}
                  onClick={() => (teamPath ? (sharing ? setSharing(false) : openShare()) : goTeam())}
                >
                  <Icon name="users" />
                </button>
                <button className="btn subtle sm" title="编辑" onClick={onEdit}><Icon name="pencil" /></button>
                <button className="btn subtle sm" title="删除" onClick={() => setConfirming(true)}><Icon name="trash" /></button>
              </>
            )
          )}
        </div>
      </div>

      {/* 共享面板：选档位 → 写进 team.yaml；已共享的可下发授权（装队友公钥）或撤销。 */}
      {sharing && local && !readonly && (
        <div className="share-panel">
          {shareErr && <div className="import-err">{shareErr}</div>}
          {shared ? (
            <>
              <div className="share-t">
                已共享给 <strong>{sharedTeam}</strong> —— 队友能看到这台机的拓扑了。
                但要让他们<strong>真能登进去</strong>，还需下发授权（在这台机上为每位成员建独立账号 + 装其公钥）。
              </div>
              <div className="share-row">
                <button className="btn primary sm" disabled={busy} onClick={onProvision}>
                  <Icon name="key" />下发授权
                </button>
                <button className="btn subtle sm" disabled={busy} onClick={openShare}>改授权</button>
                <button className="btn subtle sm" disabled={busy} onClick={doUnshare}>撤销共享</button>
                <button className="btn subtle sm" onClick={() => setSharing(false)}>收起</button>
              </div>
            </>
          ) : (
            <>
              <div className="share-t">
                共享的是<strong>拓扑</strong>（怎么到达这台机），<strong>不是凭据</strong> —— 你的密码/私钥永不出本机。
                给每个<strong>角色</strong>选开放档位（RBAC：不同角色不同权限）：
              </div>
              <div className="grant-rows">
                {roles.map((r) => (
                  <div key={r} className="grant-row">
                    <span className="grant-role">{r}</span>
                    <div className="seg sm">
                      {[0, 1, 2].map((t) => (
                        <button
                          key={t}
                          className={(grants[r] ?? defaultTier(r)) === t ? "on" : ""}
                          onClick={() => setGrants((g) => ({ ...g, [r]: t }))}
                        >
                          {t}·{TIER_SHORT[t]}
                        </button>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
              <div className="share-row">
                <button className="btn primary sm" disabled={busy || !myName} onClick={doShare}>
                  <Icon name="users" />{busy ? "共享中…" : "共享给团队"}
                </button>
                <button className="btn subtle sm" onClick={() => setSharing(false)}>取消</button>
              </div>
            </>
          )}
        </div>
      )}
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
