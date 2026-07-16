import { useEffect, useRef, useState } from "react";

import { data, type AclPlan, type GitStatus, type TeamView } from "../data";
import { TeamGraph } from "../components/TeamGraph";
import { Icon } from "../icons";

// 「团队」。一份共享 team.yaml = 成员公钥 + 共享机器（拓扑 + 开放档位），协调器极轻。
//   消费侧：加载 → 团队机作为只读节点进列表，你用自己的凭据连。
//   贡献侧：建团队 / 邀成员；机器在「服务器」页按档位共享。
//   授权侧：tier → Tailscale ACL 计划（只出计划，不替用户改 tailnet —— 责任为门）。
export function Team({
  reload,
  goServers,
  goSettings,
  teamPath,
  setTeamPath,
}: {
  reload: () => Promise<void> | void;
  goServers: () => void;
  goSettings: () => void;
  teamPath: string;
  setTeamPath: (p: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [done, setDone] = useState<{ team: string; added: number; skipped: string[] } | null>(null);
  const [plan, setPlan] = useState<AclPlan | null>(null);
  const [copied, setCopied] = useState(false);
  const [cfg, setCfg] = useState<TeamView | null>(null);

  // 建团队 / 邀成员
  const [creating, setCreating] = useState(false);
  const [teamName, setTeamName] = useState("");
  const [myName, setMyName] = useState("");
  const [pubkeys, setPubkeys] = useState<string[]>([]);
  const [pubkey, setPubkey] = useState("");
  const [addingMember, setAddingMember] = useState(false);
  const [newMember, setNewMember] = useState("");
  const [newKey, setNewKey] = useState("");
  const [newRole, setNewRole] = useState("member");
  const [newIdentity, setNewIdentity] = useState("");

  // GitHub org 绑定（花名册自动导出）
  const [ghOrg, setGhOrg] = useState("");
  const [ghToken, setGhToken] = useState("");
  const [ghBinding, setGhBinding] = useState(false); // 展开绑定表单
  const [ghSync, setGhSync] = useState<{ count: number; with_keys: number } | null>(null);

  // 验证过的团队身份（tailnet SSO）。login 为空 = 未连 tailnet。
  const [ident, setIdent] = useState<{ login: string; display: string; name: string }>({ login: "", display: "", name: "" });
  const verified = !!ident.login;

  // git 同步（配置即代码：团队配置放 git 仓库，每人维护自己那段）
  const [git, setGit] = useState<GitStatus | null>(null);
  const [gitLog, setGitLog] = useState("");
  const [cloning, setCloning] = useState(false);
  const [cloneUrl, setCloneUrl] = useState("");
  const [tick, setTick] = useState(0); // 触发刷新

  useEffect(() => {
    data.myPubkeys().then((k) => { setPubkeys(k); setPubkey(k[0] || ""); }).catch(() => {});
    data.tailnetIdentity().then((id) => {
      setIdent(id);
      if (id.name) setMyName(id.name); // 验证身份 → 用它派生的账号名（不再自填）
    }).catch(() => {});
  }, []);
  useEffect(() => {
    if (!teamPath) { setCfg(null); setGit(null); return; }
    data.readTeamView(teamPath).then(setCfg).catch(() => setCfg(null));
    data.teamGitStatus(teamPath).then(setGit).catch(() => setGit(null));
  }, [teamPath, done, tick]);

  // 读侧积极自动：进团队时后台 ff-only 拉一次（安全：拉不动就静默停，不自动 merge）。
  const autoPulled = useRef("");
  useEffect(() => {
    if (!teamPath || autoPulled.current === teamPath) return;
    autoPulled.current = teamPath;
    data.teamGitStatus(teamPath).then((st) => {
      if (st.is_repo && st.has_remote) {
        data.teamGitPull(teamPath).then(() => setTick((t) => t + 1)).catch(() => {});
      }
    }).catch(() => {});
  }, [teamPath]);

  const gitPull = async () => {
    setBusy(true); setErr(""); setGitLog("");
    try {
      setGitLog(await data.teamGitPull(teamPath) || "已是最新");
      await load(); // 拉到新机器 → 重新加载进列表
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); setTick((t) => t + 1); }
  };

  const gitPush = async () => {
    setBusy(true); setErr(""); setGitLog("");
    try {
      setGitLog(await data.teamGitPush(teamPath, `chore(team): 更新 ${cfg?.team ?? "team"} 配置`));
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); setTick((t) => t + 1); }
  };

  const doClone = async () => {
    if (!cloneUrl.trim()) { setErr("请填团队仓库地址"); return; }
    const dir = await data.pickDir();
    if (!dir) return;
    setBusy(true); setErr("");
    try {
      const repoName = cloneUrl.trim().replace(/\.git$/, "").split("/").pop() || "team";
      const p = await data.teamGitClone(cloneUrl.trim(), `${dir}/${repoName}`);
      setTeamPath(p);
      setCloning(false); setDone(null); setPlan(null);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); }
  };

  const pick = async () => {
    setErr("");
    const p = await data.pickTeamFile();
    if (p) { setTeamPath(p); setDone(null); setPlan(null); }
  };

  const load = async () => {
    if (!teamPath) { setErr("先选择一个 team.yaml"); return; }
    setBusy(true); setErr("");
    try {
      const r = await data.loadTeam(teamPath);
      await reload();
      setDone({ team: r.team, added: r.added, skipped: r.skipped });
      setPlan(await data.compileAcl(teamPath));
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); }
  };

  const create = async () => {
    if (!teamName.trim()) { setErr("请填团队名"); return; }
    if (!myName.trim()) { setErr("请填你的用户名（会成为各共享机上你的账号名）"); return; }
    const p = await data.pickTeamSavePath();
    if (!p) return;
    setBusy(true); setErr("");
    try {
      await data.createTeam(p, teamName.trim(), myName.trim(), pubkey.trim());
      setTeamPath(p);
      setCreating(false);
      setDone(null); setPlan(null);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); }
  };

  const addMember = async () => {
    if (!newMember.trim() || !newKey.trim()) { setErr("成员名与公钥都要填"); return; }
    setBusy(true); setErr("");
    try {
      setCfg(await data.addMember(teamPath, newMember.trim(), newKey.trim(), newRole, newIdentity.trim()));
      setNewMember(""); setNewKey(""); setNewIdentity(""); setAddingMember(false);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); }
  };

  // 绑定 org（默认角色映射：core-team→core，其余→member）+ 立即同步一次。
  const bindAndSync = async () => {
    if (!ghOrg.trim()) { setErr("填 GitHub org 名"); return; }
    setBusy(true); setErr("");
    try {
      await data.bindGithub(teamPath, ghOrg.trim(), { "core-team": "core", "*": "member" });
      const r = await data.syncGithub(teamPath, ghToken.trim() || undefined);
      setGhSync({ count: r.count, with_keys: r.with_keys });
      setCfg(await data.readTeamView(teamPath));
      await reload();
      setGhBinding(false);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); }
  };

  const resync = async () => {
    setBusy(true); setErr("");
    try {
      const r = await data.syncGithub(teamPath, ghToken.trim() || undefined);
      setGhSync({ count: r.count, with_keys: r.with_keys });
      setCfg(await data.readTeamView(teamPath));
      await reload();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); }
  };

  const policyJson = plan
    ? JSON.stringify(
        {
          tagOwners: Object.fromEntries(plan.tag_owners.map((t) => [t, plan.groups.map(([g]) => g)])),
          groups: Object.fromEntries(plan.groups.map(([g, ms]) => [g, ms])),
          ssh: plan.ssh.map((r) => ({
            action: r.action, src: r.src, dst: r.dst, users: r.users,
            ...(r.check_period ? { checkPeriod: r.check_period } : {}),
          })),
        }, null, 2)
    : "";

  const copy = async () => {
    await navigator.clipboard.writeText(policyJson);
    setCopied(true);
    setTimeout(() => setCopied(false), 1600);
  };

  return (
    <div className="wrap">
      <header className="page-head">
        <h1>团队</h1>
        <p>团队是一份共享配置 + 每人登记自己的设备。共享的是<strong>拓扑</strong>，不是凭据 —— 你的密码和私钥永不出本机。</p>
      </header>

      {/* 身份条：团队操作绑定验证过的 SSO 身份（防冒名）。未连 tailnet 则未验证。 */}
      <div className={"ident-bar" + (verified ? " ok" : "")}>
        <Icon name={verified ? "check" : "alert"} />
        {verified ? (
          <span>已验证身份 <strong>{ident.display || ident.login}</strong>（{ident.login}）· 账号名 <code>{ident.name}</code></span>
        ) : (
          <span>未验证身份 —— 团队操作应绑定真实身份。<a onClick={goSettings} style={{ cursor: "pointer" }}>连接 tailnet 登录</a>后即以 SSO 身份认领，防止冒名。</span>
        )}
      </div>

      {err && <div className="import-err" style={{ marginBottom: 12 }}>{err}</div>}

      {/* ① 团队配置：新建 或 选一份现有的 */}
      <article className="card open">
        <div className="cfg-head">
          <div className="srv-title"><span className="srv-name" style={{ fontSize: 17 }}>团队配置</span></div>
          {cfg && <span className="badge accent"><Icon name="users" />{cfg.team}</span>}
        </div>
        <div className="cfg-body">
          {creating ? (
            <>
              <div className="row2">
                <div className="field">
                  <label>团队名</label>
                  <div className="inp"><Icon name="users" />
                    <input value={teamName} onChange={(e) => setTeamName(e.target.value)} placeholder="如 neuroai" autoComplete="off" />
                  </div>
                </div>
                <div className="field">
                  <label>你的用户名</label>
                  <div className="inp"><Icon name="user" />
                    <input value={myName} onChange={(e) => setMyName(e.target.value)} placeholder="如 alice（小写）" autoComplete="off" />
                  </div>
                </div>
              </div>
              <div className="field">
                <label>你的公钥（公钥非机密，可以共享；私钥永不）</label>
                <div className="inp"><Icon name="key" />
                  {pubkeys.length > 0 ? (
                    <select value={pubkey} onChange={(e) => setPubkey(e.target.value)}>
                      {pubkeys.map((k) => <option key={k} value={k}>{k.slice(0, 60)}…</option>)}
                    </select>
                  ) : (
                    <input value={pubkey} onChange={(e) => setPubkey(e.target.value)} placeholder="ssh-ed25519 AAAA…（本机 ~/.ssh 里没找到）" />
                  )}
                </div>
              </div>
              <div className="cred-foot">
                <button className="btn primary sm" disabled={busy} onClick={create}><Icon name="save" />新建并保存</button>
                <button className="btn subtle sm" onClick={() => setCreating(false)}>取消</button>
              </div>
            </>
          ) : cloning ? (
            <>
              <div className="field">
                <label>团队仓库地址（配置即代码：team.yaml 放 git，天然有历史与 review）</label>
                <div className="inp"><Icon name="network" />
                  <input value={cloneUrl} onChange={(e) => setCloneUrl(e.target.value)} placeholder="git@github.com:lab/team-config.git" autoComplete="off" />
                </div>
              </div>
              <div className="cred-foot">
                <button className="btn primary sm" disabled={busy} onClick={doClone}>
                  <Icon name="upload" />{busy ? "克隆中…" : "克隆并选目录"}
                </button>
                <button className="btn subtle sm" onClick={() => setCloning(false)}>取消</button>
              </div>
            </>
          ) : (
            <>
              <div className="field">
                <label>当前 team.yaml</label>
                <div className="inp" style={{ cursor: "pointer" }} onClick={pick}>
                  <Icon name="folder" />
                  <input value={teamPath} placeholder="选择一份 team.yaml…" readOnly style={{ cursor: "pointer" }} />
                </div>
              </div>

              {/* git 同步条：拉队友的更新 / 推我的贡献 */}
              {git?.is_repo && (
                <div className="git-bar">
                  <span className="git-branch"><Icon name="network" />{git.branch}</span>
                  {git.dirty && <span className="git-dirty">有未推送的改动</span>}
                  {!git.has_remote && <span className="git-dirty">无 remote · 推不出去</span>}
                  <button className="btn subtle sm" disabled={busy || !git.has_remote} onClick={gitPull}>
                    <Icon name="refresh" />拉取
                  </button>
                  <button className="btn subtle sm" disabled={busy || !git.has_remote} onClick={gitPush}>
                    <Icon name="upload" />推送
                  </button>
                </div>
              )}
              {gitLog && <pre className="git-log">{gitLog}</pre>}

              {done && (
                <div className="team-done">
                  <Icon name="check" />
                  <span>
                    已加载团队 <strong>{done.team}</strong>：新增 {done.added} 台
                    {done.skipped.length > 0 && `，跳过 ${done.skipped.length} 台同名（${done.skipped.join("、")}）`}
                  </span>
                </div>
              )}
              <div className="cred-foot">
                <button className="btn primary sm" disabled={busy || !teamPath} onClick={load}>
                  <Icon name="users" />{busy ? "加载中…" : "加载"}
                </button>
                <button className="btn subtle sm" onClick={pick}>选择文件</button>
                <button className="btn subtle sm" onClick={() => { setCloning(true); setErr(""); }}>从 git 克隆</button>
                <button className="btn subtle sm" onClick={() => { setCreating(true); setErr(""); }}>新建团队</button>
                {done && <button className="btn subtle sm" onClick={goServers}>去服务器页</button>}
              </div>
            </>
          )}
        </div>
      </article>

      {/* ② 成员：公钥会被下发进各共享机的独立账号（身份到人） */}
      {cfg && !creating && (
        <article className="card open" style={{ marginTop: 16 }}>
          <div className="cfg-head">
            <div className="srv-title"><span className="srv-name" style={{ fontSize: 17 }}>成员</span></div>
            <span className="badge">{cfg.members.length} 人</span>
          </div>
          <div className="cfg-body">
            <p className="acl-intro">
              成员的公钥会在「下发授权」时装进各共享机上<strong>他自己的账号</strong> —— 身份到人，不用共享账号（否则操作追踪链会断）。
            </p>
            <div className="mem-list">
              {cfg.members.map((m) => (
                <div key={m.name} className="mem">
                  <span className="avatar" style={{ width: 26, height: 26, fontSize: 11 }}>{m.name.slice(0, 1).toUpperCase()}</span>
                  <span className="mem-name">{m.name}</span>
                  <span className={"role-tag " + m.role}>{m.role}</span>
                  {m.identity ? (
                    <span className="mem-id verified" title={m.identity}><Icon name="check" />{m.identity}</span>
                  ) : (
                    <span className="mem-id" title="未绑定 SSO 身份">未验证</span>
                  )}
                </div>
              ))}
            </div>

            {addingMember ? (
              <div style={{ marginTop: 14 }}>
                <div className="row2">
                  <div className="field">
                    <label>成员名（= 各机上的账号名）</label>
                    <div className="inp"><Icon name="user" />
                      <input value={newMember} onChange={(e) => setNewMember(e.target.value)} placeholder="小写字母/数字/_/-" autoComplete="off" />
                    </div>
                  </div>
                  <div className="field">
                    <label>他的 SSO 身份（邮箱 —— 声明「谁是这个成员」，防冒名的锚）</label>
                    <div className="inp"><Icon name="user" />
                      <input value={newIdentity} onChange={(e) => setNewIdentity(e.target.value)} placeholder="alice@example.com" autoComplete="off" />
                    </div>
                  </div>
                </div>
                <div className="field">
                  <label>他的公钥</label>
                  <div className="inp"><Icon name="key" />
                    <input value={newKey} onChange={(e) => setNewKey(e.target.value)} placeholder="ssh-ed25519 AAAA…" autoComplete="off" />
                  </div>
                </div>
                <div className="field">
                  <label>角色（决定他在各机上的权限档：core=sudo / member=受限 / pub=跳板）</label>
                  <div className="seg">
                    {cfg.roles.map((r) => (
                      <button key={r} className={newRole === r ? "on" : ""} onClick={() => setNewRole(r)}>{r}</button>
                    ))}
                  </div>
                </div>
                <div className="cred-foot">
                  <button className="btn primary sm" disabled={busy} onClick={addMember}><Icon name="plus" />加入团队</button>
                  <button className="btn subtle sm" onClick={() => setAddingMember(false)}>取消</button>
                </div>
              </div>
            ) : (
              <button className="add-row" onClick={() => { setAddingMember(true); setErr(""); }}>
                <Icon name="plus" />邀请成员
              </button>
            )}
          </div>
        </article>
      )}

      {/* GitHub org 花名册：绑定后成员/公钥/角色自动导出，新人进 org 自动加入 */}
      {cfg && !creating && (
        <article className="card open" style={{ marginTop: 16 }}>
          <div className="cfg-head">
            <div className="srv-title">
              <span className="srv-name" style={{ fontSize: 17 }}>GitHub 花名册</span>
              <span className="badge">自动</span>
            </div>
          </div>
          <div className="cfg-body">
            <p className="acl-intro">
              绑定 GitHub org 后，<strong>成员、公钥、角色全自动导出</strong> —— 不用每人手动登记。
              新人进 org、下次同步就自动出现（公钥拉自 <code>github.com/&lt;user&gt;.keys</code>）。
              角色映射：<code>core-team</code> → core，其余 → member。
            </p>
            {ghBinding ? (
              <>
                <div className="row2">
                  <div className="field">
                    <label>GitHub org 名</label>
                    <div className="inp"><Icon name="network" />
                      <input value={ghOrg} onChange={(e) => setGhOrg(e.target.value)} placeholder="如 neuroai-lab" autoComplete="off" />
                    </div>
                  </div>
                  <div className="field">
                    <label>Token（私有 org 才需要，公开 org 留空）</label>
                    <div className="inp"><Icon name="key" />
                      <input value={ghToken} type="password" onChange={(e) => setGhToken(e.target.value)} placeholder="ghp_…（可选）" autoComplete="off" />
                    </div>
                  </div>
                </div>
                <div className="cred-foot">
                  <button className="btn primary sm" disabled={busy} onClick={bindAndSync}><Icon name="refresh" />{busy ? "同步中…" : "绑定并同步"}</button>
                  <button className="btn subtle sm" onClick={() => setGhBinding(false)}>取消</button>
                </div>
              </>
            ) : (
              <div className="share-row">
                <button className="btn subtle sm" onClick={() => { setGhBinding(true); setErr(""); }}><Icon name="network" />绑定 org</button>
                <button className="btn subtle sm" disabled={busy} onClick={resync}><Icon name="refresh" />重新同步</button>
                {ghSync && <span className="save-note">已同步 {ghSync.count} 名成员（{ghSync.with_keys} 有公钥）</span>}
              </div>
            )}
          </div>
        </article>
      )}

      {/* ③ 拓扑图：分文件 + RBAC 后数据天然成图 —— 谁把什么算力、以什么权限、给了谁 */}
      {cfg && !creating && (cfg.machines.length > 0 || cfg.members.length > 1) && (
        <article className="card open" style={{ marginTop: 16 }}>
          <div className="cfg-head">
            <div className="srv-title">
              <span className="srv-name" style={{ fontSize: 17 }}>团队拓扑</span>
              <span className="badge">{cfg.members.length} 人 · {cfg.machines.length} 机</span>
            </div>
          </div>
          <div className="cfg-body">
            <p className="acl-intro">谁把什么算力、以什么权限、给了谁 —— 悬停看细节。这就是团队的织物。</p>
            <TeamGraph view={cfg} me={ident.login || myName} />
          </div>
        </article>
      )}

      {/* ④ 授权计划：RBAC → Tailscale ACL（终态；当前可先用「下发授权」走 authorized_keys） */}
      {plan && (
        <article className="card open" style={{ marginTop: 16 }}>
          <div className="cfg-head">
            <div className="srv-title">
              <span className="srv-name" style={{ fontSize: 17 }}>授权计划</span>
              <span className="badge">Tailscale ACL</span>
            </div>
          </div>
          <div className="cfg-body">
            <p className="acl-intro">
              把每台机的 <strong>RBAC 授权</strong>（角色→档位）编译成 Tailscale policy（终态方案）。
              <strong>我们不会替你改 tailnet</strong> —— 请 review 后自己贴进团队 policy。
              还没上 tailnet 的话，用「服务器」页每台机的<strong>下发授权</strong>（直接装公钥）即可先跑通。
            </p>

            <div className="acl-sec-t">每台机要做什么</div>
            <div className="acl-machines">
              {plan.machines.map((m) => (
                <div key={m.name} className="acl-machine">
                  <div className="acl-m-head">
                    <span className="acl-m-name">{m.name}</span>
                    <span className="acl-grants">{m.grants_desc || "未授权"}</span>
                    <span className="acl-m-host">{m.host} · {m.owner}</span>
                  </div>
                  <code className="acl-cmd">{m.command}</code>
                  <div className="acl-harden"><Icon name="shield" />{m.hardening}</div>
                </div>
              ))}
            </div>

            <div className="acl-sec-t">
              Policy 片段
              <button className="acl-copy" onClick={copy}>
                <Icon name={copied ? "check" : "file"} />{copied ? "已复制" : "复制"}
              </button>
            </div>
            <pre className="acl-json">{policyJson}</pre>

            <div className="acl-notes">
              {plan.notes.map((n, i) => (
                <div key={i} className="acl-note"><Icon name="alert" /><span>{n}</span></div>
              ))}
            </div>
          </div>
        </article>
      )}

      <div className="team-why">
        <div className="team-why-head"><Icon name="network" /><span>它是怎么运作的</span></div>
        <ul>
          <li><strong>配置即代码</strong> · 团队约定一份 <code>team.yaml</code>，每人往里登记自己的设备与授权，协调器轻到几乎没有。</li>
          <li><strong>共享拓扑，不共享凭据</strong> · 给团队的是「怎么到达这台机」；密码和私钥永不出本机。下发的只是队友的<strong>公钥</strong>。</li>
          <li><strong>身份到人</strong> · 每位成员在共享机上有自己的账号 —— 不用共享账号，否则「谁做了什么」的追踪链会断。</li>
          <li><strong>档位由机器主人定</strong> · 开多大权你说了算，你也为屋内的权限边界负责。</li>
        </ul>
      </div>
    </div>
  );
}
