import { useEffect, useRef, useState } from "react";

import { TailnetPanel } from "../components/TailnetPanel";
import { data, type AclPlan, type GitStatus, type TeamView } from "../data";
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
  onReauth,
}: {
  reload: () => Promise<void> | void;
  goServers: () => void;
  goSettings: () => void;
  teamPath: string;
  setTeamPath: (p: string) => void;
  onReauth: () => void; // 重新授权 GitHub（退出重新登录，拿 repo 权限）
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

  // GitHub 花名册同步结果(绑定由登录选 org 自动完成,不再有手动表单)
  const [ghSync, setGhSync] = useState<{ count: number; with_keys: number } | null>(null);

  // 验证过的团队身份（tailnet SSO）。login 为空 = 未连 tailnet。
  const [ident, setIdent] = useState<{ login: string; display: string; name: string }>({ login: "", display: "", name: "" });
  // 显示用身份（含系统 tailscale / OS 用户兜底）——只喂图上标「我」。
  const [localId, setLocalId] = useState<{ login: string; display: string; name: string }>({ login: "", display: "", name: "" });
  const verified = !!ident.login;

  // git 同步（配置即代码：团队配置放 git 仓库，每人维护自己那段）
  const [git, setGit] = useState<GitStatus | null>(null);
  const [gitLog, setGitLog] = useState("");
  const [cloning, setCloning] = useState(false);
  const [cloneUrl, setCloneUrl] = useState("");
  const [tick, setTick] = useState(0); // 触发刷新

  // GitHub 组织（登录选定，团队 = 该 org 的约定仓库 <org>/ait-team）。可切换。
  const [curOrg, setCurOrg] = useState("");
  const [orgs, setOrgs] = useState<string[] | null>(null); // null=未展开列表
  const [orgBusy, setOrgBusy] = useState(false);
  const [manualOrg, setManualOrg] = useState(""); // 手动输入（API 没列出的 org 也能激活）
  const [reauth, setReauth] = useState(false); // 建仓库缺 repo 权限 → 提示重新授权

  useEffect(() => {
    data.myPubkeys().then((k) => { setPubkeys(k); setPubkey(k[0] || ""); }).catch(() => {});
    data.tailnetIdentity().then((id) => {
      setIdent(id);
      if (id.name) setMyName(id.name); // 验证身份 → 用它派生的账号名（不再自填）
    }).catch(() => {});
    data.localIdentity().then(setLocalId).catch(() => {});
    data.ghState().then((s) => setCurOrg(s.org)).catch(() => {});
  }, []);

  // 切换组织：clone/绑定那个 org 的约定仓库，设为当前团队。
  const switchOrg = async (org: string) => {
    if (org === curOrg) { setOrgs(null); return; }
    setOrgBusy(true); setErr("");
    try {
      const r = await data.ghActivateOrg(org);
      if (r.path) setTeamPath(r.path);
      setCurOrg(org);
      setOrgs(null);
      await reload();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setOrgBusy(false);
    }
  };
  const openOrgList = async () => {
    if (orgs) { setOrgs(null); return; }
    try { setOrgs(await data.ghOrgs()); } catch (e) { setErr(e instanceof Error ? e.message : String(e)); }
  };

  // 为当前 org 生成团队仓库初始模板（team.yaml + members/ + README + git init），设为当前团队。
  const initTemplate = async () => {
    if (!curOrg) return;
    setOrgBusy(true); setErr("");
    try {
      const p = await data.ghInitTeam(curOrg);
      setTeamPath(p);
      setTick((t) => t + 1); // 刷新 git 状态（现在是个 repo 了）
      setGitLog(
        `已生成 ${curOrg} 的团队配置模板（team.yaml + members/ + README，已 git init）。\n` +
        `接着建 GitHub 私有仓库 ${curOrg}/ait-team 并推送：\n` +
        `  git remote add origin git@github.com:${curOrg}/ait-team.git\n` +
        `  git branch -M main && git push -u origin main\n` +
        `或用 gh：gh repo create ${curOrg}/ait-team --private --source=. --push`,
      );
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setOrgBusy(false);
    }
  };
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

  // 打开 GitHub 新建仓库页（org + ait-team + private 预填）——建仓库需写权限，这一步在网页做。
  const openNewRepo = async () => {
    const u = await data.ghNewRepoUrl(curOrg);
    if (u) data.openUrl(u);
  };
  // 一键建仓库并推送:API 建 <org>/ait-team + 系统 git 连远程推送。缺 repo 权限 → 提示重新授权。
  const connectPush = async () => {
    setBusy(true); setErr(""); setGitLog(""); setReauth(false);
    try {
      setGitLog(await data.ghPushTeam(curOrg, teamPath));
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (msg.startsWith("REAUTH:")) { setReauth(true); setErr(msg.slice(7)); }
      else setErr(msg);
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
  // 同步花名册。token 不用填 —— 后端自动用登录时存进钥匙串的。
  const resync = async () => {
    setBusy(true); setErr("");
    try {
      const r = await data.syncGithub(teamPath);
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

      {/* 身份条:团队操作绑定验证身份(GitHub 登录优先,tailnet SSO 备选)防冒名。 */}
      <div className={"ident-bar" + (verified ? " ok" : "")}>
        <Icon name={verified ? "check" : "alert"} />
        {verified ? (
          <span>已验证身份 <strong>{ident.display || ident.login}</strong>（{ident.login}）· 账号名 <code>{ident.name}</code></span>
        ) : (
          <span>未验证身份 —— 用 GitHub 登录(侧栏头像菜单「连接 GitHub」)或<a onClick={goSettings} style={{ cursor: "pointer" }}>连接 tailnet</a>后,团队操作即以该身份认领,防止冒名。</span>
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
              {/* GitHub 组织 = 团队。团队配置 = 该 org 的约定仓库 <org>/ait-team。 */}
              {curOrg && (
                <div className="org-bar">
                  <div className="org-cur">
                    <Icon name="users" />
                    <div>
                      <div className="org-name">{curOrg}</div>
                      <div className="org-sub">团队配置仓库 <code>{curOrg}/ait-team</code></div>
                    </div>
                  </div>
                  <button className="btn subtle sm" disabled={orgBusy} onClick={openOrgList}>
                    <Icon name="updown" />{orgBusy ? "切换中…" : "切换组织"}
                  </button>
                </div>
              )}
              {orgs && (
                <div className="org-list">
                  {orgs.length === 0 && (
                    <div className="org-empty">
                      API 没列到组织 —— GitHub App 只看得到已安装它的 org(换经典 OAuth App 可列全)。下面手动输入也能用。
                    </div>
                  )}
                  {orgs.map((o) => (
                    <button key={o} className={"org-opt" + (o === curOrg ? " on" : "")} disabled={orgBusy} onClick={() => switchOrg(o)}>
                      <Icon name="users" />{o}{o === curOrg && " · 当前"}
                    </button>
                  ))}
                  {/* 兜底:直接输 org 名。激活只需名字 —— clone 走你自己的 git 权限,不受 OAuth 授权范围限制。 */}
                  <div className="org-manual">
                    <input
                      value={manualOrg}
                      onChange={(e) => setManualOrg(e.target.value)}
                      placeholder="没列出?直接输入组织名…"
                      onKeyDown={(e) => e.key === "Enter" && manualOrg.trim() && switchOrg(manualOrg.trim())}
                    />
                    <button className="btn subtle sm" disabled={!manualOrg.trim() || orgBusy} onClick={() => switchOrg(manualOrg.trim())}>
                      {orgBusy ? "…" : "使用"}
                    </button>
                  </div>
                  {/* 没列出的 org 多半开了第三方 App 限制:owner 去 Grant、成员去 Request(GitHub 页面完成)。 */}
                  <button className="org-authlink" onClick={async () => { const u = await data.ghAuthorizeUrl(); if (u) data.openUrl(u); }}>
                    组织没列出? 在 GitHub 授权页 Grant(owner)/ Request(成员) →
                  </button>
                </div>
              )}

              {/* 约定仓库不存在 → 只显示成员，机器共享未生效。可一键生成初始模板。 */}
              {curOrg && git && !git.is_repo && (
                <div className="org-hint">
                  <div>组织 <b>{curOrg}</b> 还没有团队仓库 <code>{curOrg}/ait-team</code> —— 现在只显示成员。
                  机器共享要先有这个仓库。</div>
                  <button className="btn primary sm" style={{ marginTop: 10 }} disabled={orgBusy} onClick={initTemplate}>
                    <Icon name="save" />{orgBusy ? "生成中…" : `为 ${curOrg} 生成初始配置模板`}
                  </button>
                </div>
              )}

              {/* 高级：直接指定一份本地 team.yaml（一般不需要，团队跟随上面的组织）。 */}
              {/* 高级:主路是「登录选 org 自动接管一切」;这些手动路径收在这里备用。 */}
              <details className="team-adv">
                <summary>高级 · 手动管理 team.yaml(一般不需要,团队跟随上面的组织)</summary>
                <div className="field" style={{ marginTop: 8 }}>
                  <div className="inp" style={{ cursor: "pointer" }} onClick={pick}>
                    <Icon name="folder" />
                    <input value={teamPath} placeholder="选择一份 team.yaml…" readOnly style={{ cursor: "pointer" }} />
                  </div>
                </div>
                <div className="cred-foot" style={{ marginTop: 8 }}>
                  <button className="btn subtle sm" onClick={pick}>选择文件</button>
                  <button className="btn subtle sm" onClick={() => { setCloning(true); setErr(""); }}>从 git 克隆</button>
                  <button className="btn subtle sm" onClick={() => { setCreating(true); setErr(""); }}>新建团队</button>
                </div>
              </details>

              {/* 本地仓库但还没连 GitHub:一键建仓库(API)+ 推送(git)。缺 repo 权限时提示重新授权。 */}
              {git?.is_repo && !git.has_remote && curOrg && (
                <div className="git-connect">
                  <div className="git-connect-hd">还没连 GitHub —— 把 <code>{curOrg}/ait-team</code> 推上去,队友才拉得到:</div>
                  <div className="git-connect-row">
                    <button className="btn primary sm" disabled={busy} onClick={connectPush}>
                      <Icon name="upload" />{busy ? "建仓库并推送…" : "一键建仓库并推送"}
                    </button>
                    <button className="btn subtle sm" disabled={busy} onClick={openNewRepo}>手动在网页建</button>
                  </div>
                  {reauth ? (
                    <div className="git-connect-tip">
                      需要仓库权限才能自动建仓库。<button className="org-authlink" onClick={onReauth}>重新授权 GitHub(退出重新登录)→</button> 或点上面「手动在网页建」。
                    </div>
                  ) : (
                    <div className="git-connect-tip">建仓库用你的 GitHub 授权、推送走你自己的 git 凭据,全程不碰命令行。</div>
                  )}
                </div>
              )}

              {/* git 同步条：拉队友的更新 / 推我的贡献 */}
              {git?.is_repo && git.has_remote && (
                <div className="git-bar">
                  <span className="git-branch"><Icon name="network" />{git.branch}</span>
                  {git.dirty && <span className="git-dirty">有未推送的改动</span>}
                  <button className="btn subtle sm" disabled={busy} onClick={gitPull}>
                    <Icon name="refresh" />拉取
                  </button>
                  <button className="btn subtle sm" disabled={busy} onClick={gitPush}>
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

      {/* 团队网络（tailnet）= 可达性地基。大家在同一张 tailnet,直连/经门才谈得上。
          从「设置」深处提到团队流程里 —— 这是团队一等基建,不是技术开关。 */}
      {cfg && !creating && (
        <section style={{ marginTop: 16 }}>
          <div className="acl-sec-t" style={{ marginBottom: 8 }}>团队网络 · Tailnet（可达性地基）</div>
          <TailnetPanel teamTailnet={cfg.tailnet} />
        </section>
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
              成员、公钥、角色从 GitHub org <strong>全自动导出</strong> —— 登录选组织即已绑定,不用每人手动登记。
              新人进 org、下次同步就自动出现（公钥拉自 <code>github.com/&lt;user&gt;.keys</code>）。
              角色映射在 team.yaml 的 <code>role_map</code>(默认全员 member,可加 <code>core-team: core</code>)。
            </p>
            <div className="share-row">
              <button className="btn subtle sm" disabled={busy} onClick={resync}><Icon name="refresh" />重新同步</button>
              {ghSync && <span className="save-note">已同步 {ghSync.count} 名成员（{ghSync.with_keys} 有公钥）</span>}
            </div>
          </div>
        </article>
      )}

      {/* ③ 拓扑图：分文件 + RBAC 后数据天然成图 —— 谁把什么算力、以什么权限、给了谁 */}

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
