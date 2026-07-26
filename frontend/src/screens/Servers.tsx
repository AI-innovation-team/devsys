import { useEffect, useState } from "react";

import { Me, Server } from "../api";
import {
  data, supportsLocalTopology, DEFAULT_SHARING,
  type ServerInput, type SshHost, type Sharing, type ShareLimit, type Isolation, type HostProbe,
} from "../data";
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
  const [roles, setRoles] = useState<string[]>(["core", "member", "pub"]);
  // 我的成员名 = **统一身份锚**(GitHub 登录名派生的账号句柄)。
  // 曾用 me.user(本地 profile 文件),改 GitHub 登录后它恒为空 → 共享按钮永远禁用、点了没反应。
  const [myName, setMyName] = useState("");
  useEffect(() => {
    data.tailnetIdentity()
      .then((id) => setMyName(id.name || me?.user || ""))
      .catch(() => setMyName(me?.user || ""));
  }, [me?.user]);
  useEffect(() => {
    if (!teamPath) return;
    data.readTeamView(teamPath)
      .then((v) => setRoles(v.roles))
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
  // grants：每个角色开的档位。默认 core=2 / member=1 / pub=0（缺省 1）。
  const defaultTier = (r: string) => (r === "core" ? 2 : r === "pub" ? 0 : 1);
  const [grants, setGrants] = useState<Record<string, number>>({});
  // 「怎么关」= 隔离方式 + 借出上限 + 点名共享的数据集。与 grants（「开多少权」）正交。
  const [iso, setIso] = useState<Isolation>("container");
  // 那台机到底支持哪几种兑现方式 —— 共享**之前**就探，别等下发才发现没装 docker。
  const [caps, setCaps] = useState<HostProbe | null>(null);
  const [capsBusy, setCapsBusy] = useState(false);
  const [cpus, setCpus] = useState("");
  const [mem, setMem] = useState("");
  const [gpus, setGpus] = useState("");
  const [dsets, setDsets] = useState<{ host: string; as: string }[]>([]);
  // 这台是「我自己的设备」还是「我贡献的服务器」？只有本人分得清 —— 由他勾。
  const [asDevice, setAsDevice] = useState(false);
  const [busy, setBusy] = useState(false);
  const [shareErr, setShareErr] = useState("");
  const [syncNote, setSyncNote] = useState(""); // 同步给团队的结果

  const shared = (s.shared_to?.length ?? 0) > 0;
  const sharedTeam = s.shared_to?.[0]?.replace(/^team:/, "") ?? "";

  // 打开面板:已共享的机器要**回填**它现有的设置，否则「改授权」会把主人配好的
  // 上限/数据集悄悄清空（改一次权限赔进去一套借出策略）。
  // 已共享的机器点「改授权」要能看见档位表 —— 面板原先只按 shared 分支渲染，
  // 于是「改授权」点了没反应（永远停在已共享那屏）。用这个标志把编辑态摘出来。
  const [editing, setEditing] = useState(false);

  // 两种兑现方式。**容器是主线,三平台通吃**（Linux / macOS / Windows 只要有 docker/podman —— 
  // 容器里永远是 Linux,宿主是什么无所谓）;裸机账号是 Linux-only 的备选,
  // 因为它要 useradd/sudoers,而且在没有 cgroups 的系统上连限额都给不了。
  const ISOS: { v: Isolation; label: string; why: (c: HostProbe | null) => string }[] = [
    {
      v: "container", label: "一人一容器",
      why: (c) => (!c ? "" : !c.engine ? "这台机没装 docker 也没装 podman" : ""),
    },
    {
      v: "account", label: "裸机账号（备选）",
      why: (c) => (!c ? "" : c.os !== "linux" ? `这一档要 useradd/sudoers，只支持 Linux（这台是 ${c.os}）`
        : !c.host_root ? "需要这台机的 root / 免密 sudo" : ""),
    },
  ];
  const isoBlocked = (v: Isolation) => ISOS.find((i) => i.v === v)!.why(caps);

  const probe = async () => {
    setCapsBusy(true); setCaps(null);
    try { setCaps(await data.probeHost(s.name)); }
    catch { /* 连不上就不显示能力条，不打断共享 */ }
    finally { setCapsBusy(false); }
  };

  const openShare = async () => {
    setEditing(true);
    void probe();
    const g: Record<string, number> = {};
    for (const r of roles) g[r] = defaultTier(r);
    setIso("account"); setCpus(""); setMem(""); setGpus(""); setDsets([]);
    setShareErr("");
    setSharing(true);
    if (!shared || !teamPath) { setGrants(g); return; }
    try {
      const v = await data.readTeamView(teamPath);
      const m = v.machines.find((x) => x.name === s.name);
      if (m) {
        for (const r of roles) if (m.grants[r] !== undefined) g[r] = m.grants[r];
        const sh = m.sharing;
        // 老 team.yaml 里的 rootless 就是现在的 container（两档已合一）
        if (sh?.isolation) setIso(sh.isolation === "account" ? "account" : "container");
        setAsDevice(!!m.is_self);
        setCpus(sh?.limit?.cpus != null ? String(sh.limit.cpus) : "");
        setMem(sh?.limit?.mem ?? "");
        setGpus(sh?.limit?.gpus ?? "");
        setDsets((sh?.data ?? []).map((d) => ({ host: d.host, as: d.as ?? "" })));
      }
    } catch { /* 读不到就用默认值 */ }
    setGrants(g);
  };

  // 面板上的输入 → 后端的 Sharing。空字段一律不写进 team.yaml。
  const buildSharing = (): Sharing => {
    if (iso === "account") return { ...DEFAULT_SHARING };
    const n = parseFloat(cpus);
    const limit: ShareLimit = {};
    if (cpus.trim() && Number.isFinite(n) && n > 0) limit.cpus = n;
    if (mem.trim()) limit.mem = mem.trim();
    if (gpus.trim()) limit.gpus = gpus.trim();
    const has = limit.cpus != null || !!limit.mem || !!limit.gpus;
    return {
      isolation: iso,
      image: "",
      limit: has ? limit : null,
      data: dsets
        .filter((d) => d.host.trim())
        .map((d) => ({ host: d.host.trim(), as: d.as.trim() || undefined, mode: "ro" })),
    };
  };

  // 贡献写在 members/<我>.yaml,**必须推到团队仓库**队友和控制面才看得到
  // (控制面每 5 分钟从仓库对账 ACL)。推失败不回滚本地,提示去团队页重试。
  const syncTeam = async (msg: string) => {
    setSyncNote("同步给团队…");
    try {
      await data.teamGitPush(teamPath, msg);
      setSyncNote("✅ 已同步给团队(控制面几分钟内生效)");
    } catch (e) {
      const m = e instanceof Error ? e.message : String(e);
      setSyncNote("⚠ 本地已保存,但推送团队仓库失败 —— 去「连接团队」页点「推送」重试。" + m.slice(0, 80));
    }
  };

  const doShare = async () => {
    setBusy(true); setShareErr(""); setSyncNote("");
    try {
      await data.shareServer(teamPath, myName, s.name, grants, buildSharing(), asDevice);
      await reload();
      setEditing(false);
      setSharing(false);
      await syncTeam(`chore(team): 共享 ${s.name}`);
    } catch (e) {
      setShareErr(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); }
  };

  const doUnshare = async () => {
    setBusy(true); setShareErr(""); setSyncNote("");
    try {
      await data.unshareServer(teamPath, myName, s.name);
      await reload();
      await syncTeam(`chore(team): 取消共享 ${s.name}`);
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
                  onClick={() => (teamPath ? (sharing ? (setSharing(false), setEditing(false)) : openShare()) : goTeam())}
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
          {syncNote && <div className="sync-note">{syncNote}</div>}
          {shared && !editing ? (
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
                <button className="btn subtle sm" onClick={() => { setEditing(false); setSharing(false); }}>收起</button>
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
              {/* ★「档」= 开多少权（上面的 grants）与「怎么关」= 隔离方式（下面）正交。
                  裸机账号给不了资源限额与目录隔离；一人一容器才能逐人兑现档位。 */}
              <label className="dev-flag">
                <input type="checkbox" checked={asDevice} onChange={(e) => setAsDevice(e.target.checked)} />
                <span>
                  这是<strong>我自己的设备</strong>（笔记本 / 台式，跟着我走）
                  <em>不勾 = 我贡献的服务器（实验室基建）。设备会挂在织物图上「我」的旁边；一个人可以有多台。</em>
                </span>
              </label>

              <div className="share-t iso-t">
                这些档位<strong>怎么兑现</strong>：
              </div>

              {/* 那台机的实际现状 —— 不可用的档位灰掉并说清缺什么，别等下发才报错 */}
              <div className="caps-bar">
                {capsBusy ? <span className="caps-probing">正在看 {s.name} 支持什么…</span> : caps ? (
                  <>
                    <span className="caps-chip ok">{caps.os}</span>
                    <span className={"caps-chip" + (caps.engine ? " ok" : "")}>
                      {caps.engine || "无容器引擎"}{caps.rootless ? " · rootless" : ""}
                    </span>
                    {caps.desktop && <span className="caps-chip">容器在 VM 里</span>}
                    {caps.ncpu > 0 && (
                      <span className="caps-chip">
                        引擎可见 {caps.ncpu} 核 / {(caps.mem_mib / 1024).toFixed(1)}G
                        {caps.desktop ? "（VM 配额）" : ""}
                      </span>
                    )}
                    {caps.host_root && <span className="caps-chip ok">有 root</span>}
                    {caps.gpu && <span className="caps-chip ok">GPU</span>}
                  </>
                ) : (
                  <button className="btn subtle sm" onClick={probe}>探测这台机支持什么</button>
                )}
              </div>

              <div className="seg sm iso-seg">
                {ISOS.map((o) => {
                  const blocked = o.why(caps);
                  return (
                    <button
                      key={o.v}
                      className={iso === o.v ? "on" : ""}
                      disabled={!!blocked}
                      title={blocked || undefined}
                      onClick={() => setIso(o.v)}
                    >
                      {o.label}
                    </button>
                  );
                })}
              </div>
              {isoBlocked(iso) && <div className="import-err">{isoBlocked(iso)}</div>}
              {caps?.install_hint && <pre className="caps-hint">{caps.install_hint}</pre>}

              <div className="iso-hint">
                {iso === "container" ? (
                  <>每人一个独立容器：档位逐人兑现、配额逐人生效、爆炸半径只有他自己。
                  队友直连他自己容器里的 sshd（<strong>各占一个高位端口</strong>，app 会自动填给他们），
                  <strong>拿不到宿主 shell</strong>、看不见你的目录。
                  <strong>三个平台通吃</strong> —— 容器里永远是 Linux，你的机器是什么无所谓；
                  也不需要这台机的 root。</>
                ) : (
                  <>每人一个宿主账号（无 sudo / 有 sudo 按档定）。简单，但<strong>没有资源限额</strong>，
                  一个人能占满整台机，也看得见你的目录。需要 Linux + root。</>
                )}
              </div>

              {iso !== "account" && (
                <div className="iso-box">
                  <div className="grant-row">
                    <span className="grant-role">借出上限</span>
                    <div className="iso-fields">
                      <input value={cpus} onChange={(e) => setCpus(e.target.value)} placeholder="CPU 核数 如 8" />
                      <input value={mem} onChange={(e) => setMem(e.target.value)} placeholder="内存 如 32g" />
                      <input value={gpus} onChange={(e) => setGpus(e.target.value)} placeholder="GPU 如 all / device=0,1" />
                    </div>
                  </div>
                  <div className="iso-note">
                    {iso === "container" ? (
                      <>这是你<strong>最多借出多少</strong> —— 落成一个父 cgroup 池，所有借用容器挂它下面，
                      开多少个容器加起来都突破不了。随时可改、可设 0 收回。
                      （GPU 是设备直通，不受父池约束：每个容器都会拿到你写的这组卡。）</>
                    ) : (
                      <>免 root 下建不了父 cgroup 池（要 root），所以这是<strong>每个容器各自的上限</strong> ——
                      N 个人最多能占到 N 倍。要硬上限得用要 root 的模式。</>
                    )}
                  </div>

                  <div className="grant-row">
                    <span className="grant-role">共享数据集</span>
                    <button className="btn subtle sm" onClick={() => setDsets((d) => [...d, { host: "", as: "" }])}>
                      + 加一个
                    </button>
                  </div>
                  {dsets.map((d, i) => (
                    <div key={i} className="iso-fields">
                      <input
                        value={d.host}
                        onChange={(e) => setDsets((a) => a.map((x, j) => (j === i ? { ...x, host: e.target.value } : x)))}
                        placeholder="宿主路径 如 /data/imagenet"
                      />
                      <input
                        value={d.as}
                        onChange={(e) => setDsets((a) => a.map((x, j) => (j === i ? { ...x, as: e.target.value } : x)))}
                        placeholder="容器内路径（留空 = 同上）"
                      />
                      <button className="btn subtle sm" onClick={() => setDsets((a) => a.filter((_, j) => j !== i))}>
                        <Icon name="x" />
                      </button>
                    </div>
                  ))}
                  <div className="iso-note">
                    只有你<strong>点名</strong>的目录会<strong>只读</strong>挂进每个容器；其余宿主目录一律不可见。
                    大数据集共用一份、不各拷 —— 计算跑到数据旁边，数据不出这台机。
                  </div>
                </div>
              )}

              <div className="share-row">
                <button className="btn primary sm" disabled={busy || !myName} onClick={doShare}>
                  <Icon name="users" />{busy ? "共享中…" : "共享给团队"}
                </button>
                <button className="btn subtle sm" onClick={() => { setEditing(false); if (!shared) setSharing(false); }}>取消</button>
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
