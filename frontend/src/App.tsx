import { useCallback, useEffect, useRef, useState } from "react";

import { api, breadcrumb, collectSlugs, DocNode, groupChildren, neighbors, Me } from "./api";
import { data } from "./data";
import { isTauri } from "./transport";
import { GithubGate } from "./components/GithubGate";
import { Sidebar } from "./components/Sidebar";
import { Admin } from "./screens/Admin";
import { Docs } from "./screens/Docs";
import { Servers } from "./screens/Servers";
import { Settings } from "./screens/Settings";
import { Team } from "./screens/Team";
import { Terminal } from "./screens/Terminal";
import { Home } from "./screens/Home";
import { Workspace, type WorkspaceHandle } from "./screens/Workspace";
import { Workspaces } from "./screens/Workspaces";

export type View = "home" | "workspaces" | "servers" | "team" | "docs" | "settings" | "admin";
export type Theme = "light" | "dark";

const VIEWS: View[] = ["home", "workspaces", "servers", "team", "docs", "settings", "admin"];

// 自包含 app（tauri）：主页(织物图) + 工作区(活终端) + 服务器 + 团队 + 设置。
// 「工作区」在本地是桌面版 <Workspace>（常驻挂载）；web 版才是门户 <Workspaces>。
const LOCAL_VIEWS: View[] = ["home", "workspaces", "servers", "team", "settings"];

function restoreView(): View {
  if (isTauri) {
    try {
      const v = localStorage.getItem("devsys.view");
      if (v && LOCAL_VIEWS.includes(v as View)) return v as View;
    } catch {}
    return "home";
  }
  try {
    const v = localStorage.getItem("devsys.view");
    if (v && VIEWS.includes(v as View)) return v as View;
  } catch {}
  return "workspaces";
}

export function App() {
  const [me, setMe] = useState<Me | null>(null);
  const [view, setViewState] = useState<View>(restoreView);
  // 侧栏默认收起、不记忆：每次进入都收起，展开只在当前会话内有效。
  const [collapsed, setCollapsed] = useState<boolean>(true);
  const [theme, setThemeState] = useState<Theme>(
    () => (document.documentElement.getAttribute("data-theme") === "dark" ? "dark" : "light"),
  );

  // 文档：层级树 + 当前项（activeDoc 为空串表示“文档首页/导览”）
  const [docTree, setDocTree] = useState<DocNode[]>([]);
  const [activeDoc, setActiveDoc] = useState("");

  // app 内终端（tauri）：非空则全屏打开该服务器的终端。web 仍走新标签页。
  const [term, setTerm] = useState<{ server: string; ws: string } | null>(null);

  // 当前团队配置（team.yaml 路径）。跨屏共享：Team 屏设定，Servers 屏据此贡献机器。
  const [teamPath, setTeamPathState] = useState<string>(() => {
    try { return localStorage.getItem("devsys.team") || ""; } catch { return ""; }
  });
  const setTeamPath = (p: string) => {
    setTeamPathState(p);
    try { p ? localStorage.setItem("devsys.team", p) : localStorage.removeItem("devsys.team"); } catch { /* ignore */ }
  };

  // 桌面「工作区」常驻挂载（切视图只 CSS 隐藏，绝不卸载 —— 卸载会关掉所有 SSH 会话）。
  // 主页点节点 / 服务器页点 SSH 都汇到这里：开出 pane 并切到工作区。
  const wsRef = useRef<WorkspaceHandle>(null);
  const openInWorkspace = useCallback((server: string) => {
    wsRef.current?.openPane(server);
    setView("workspaces");
  }, []);

  // 保险库在登录门已解锁，会话内无需再次解锁。
  // tauri：SSH 一律进「工作区」pane（可多标签/分屏/持久）；web 仍走全屏 Terminal。
  const goTerminal = (server: string, ws = "") => {
    if (isTauri) { openInWorkspace(server); return; }
    setTerm({ server, ws });
  };

  // 退出登录：清 GitHub 会话 + 关会话，回到 GitHub 登录门。保险库不锁（设备密钥托管）。
  const LOCAL_ONLY = "devsys.localonly";
  const logout = async () => {
    try { await data.ghLogout(); } catch { /* ignore */ }
    try { localStorage.removeItem(LOCAL_ONLY); } catch { /* ignore */ }
    setTeamPath(""); // 换身份 = 清当前团队；下次选 org 会重新自动绑定
    teamSynced.current = false; // 重新登录后要再对齐一次团队上下文
    setTerm(null);
    setMe(null);
    setGate("login");
  };

  // 登录门（tauri）= GitHub 身份。保险库由设备密钥在 Rust setup 里自动解锁，这里只管身份。
  // 「先进入本地」会存 localonly 标记，避免每次启动再问；用户菜单可随时重新连 GitHub。
  const [gate, setGate] = useState<"checking" | "login" | null>(isTauri ? "checking" : null);
  const openGithubLogin = () => { try { localStorage.removeItem(LOCAL_ONLY); } catch {} setGate("login"); };
  useEffect(() => {
    if (!isTauri) return;
    let localOnly = false;
    try { localOnly = localStorage.getItem(LOCAL_ONLY) === "1"; } catch { /* ignore */ }
    if (localOnly) { setGate(null); return; }
    data.ghState()
      .then((s) => setGate(s.logged_in && s.org ? null : "login"))
      .catch(() => setGate("login"));
  }, []);

  const reload = useCallback(async () => {
    try { setMe(await data.loadMe()); } catch { /* web: Caddy 外层已鉴权；tauri: 本地空清单 */ }
  }, []);
  useEffect(() => { if (!isTauri || gate === null) reload(); }, [reload, gate]);

  // GitHub 身份 + **启动即对齐团队上下文**(单活跃团队不变量的执行点,每次进 app 必跑一次):
  //   有 org → 重新激活(pull 仓库 + 同步花名册 + 折进 store,顺带清光旧团队残余);
  //   没 org 但有 teamPath(高级手动路径)→ 重新加载它(同样清残余);
  //   两者都没有 → 兜底清掉所有 team:* 机器。
  // 之前只在「teamPath 为空」时才激活 → 老状态永远轮不到清理,旧团队(如 labnet/neuroai)
  // 在服务器列表里装死 —— 这里改成无条件对齐。
  const [gh, setGh] = useState<{ login: string; org: string; logged_in: boolean }>({ login: "", org: "", logged_in: false });
  const teamSynced = useRef(false);
  useEffect(() => {
    if (!isTauri || gate !== null) return;
    data.ghState().then(async (s) => {
      setGh({ login: s.login, org: s.org, logged_in: s.logged_in });
      if (teamSynced.current) return;
      teamSynced.current = true;
      try {
        if (s.logged_in && s.org) {
          const r = await data.ghActivateOrg(s.org);
          if (r.path) setTeamPath(r.path);
        } else if (teamPath) {
          await data.loadTeam(teamPath);
        } else {
          await data.pruneTeamSources();
        }
      } catch { /* 网络失败等不阻塞进 app;下次启动再对齐 */ }
      reload();
    }).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gate]);

  // 旧密码保险库迁移：换成设备密钥后，旧库设备密钥开不动 → 提示迁移（销毁旧凭据、重建空库）。
  const [vaultLegacy, setVaultLegacy] = useState(false);
  const [migrating, setMigrating] = useState(false);
  useEffect(() => {
    if (!isTauri || gate !== null) return;
    data.vaultState().then((v) => setVaultLegacy(!!v.legacy)).catch(() => {});
  }, [gate]);
  const migrateVault = async () => {
    setMigrating(true);
    try { await data.vaultMigrate(); setVaultLegacy(false); reload(); }
    catch { /* ignore */ }
    finally { setMigrating(false); }
  };

  useEffect(() => {
    if (isTauri) return; // 自包含 app 无门户文档
    (async () => {
      try {
        const d = await api.docs();
        setDocTree(d.tree);
        // 恢复上次阅读位置；无效或未记录则停在首页（空串）
        let last = "";
        try { last = localStorage.getItem("devsys.doc") || ""; } catch { /* ignore */ }
        setActiveDoc(collectSlugs(d.tree).includes(last) ? last : "");
      } catch { setDocTree([]); }
    })();
  }, []);

  const setView = (v: View) => {
    setViewState(v);
    try { localStorage.setItem("devsys.view", v); } catch {}
  };
  const setTheme = (t: Theme) => {
    document.documentElement.setAttribute("data-theme", t);
    try { localStorage.setItem("devsys.theme", t); } catch {}
    setThemeState(t);
  };
  const toggleCollapse = () => setCollapsed((c) => !c);

  // 打开某篇文档（slug 为空串 = 文档首页）；侧栏「文档」按钮即 openDoc("")
  const openDoc = (slug: string) => {
    setActiveDoc(slug);
    setView("docs");
    try { localStorage.setItem("devsys.doc", slug); } catch {}
  };

  if (gate === "checking") return null;
  if (gate === "login") {
    return (
      <GithubGate
        onDone={() => setGate(null)}
        onSkip={() => { try { localStorage.setItem(LOCAL_ONLY, "1"); } catch {} setGate(null); }}
      />
    );
  }

  if (term) return <Terminal server={term.server} ws={term.ws} onBack={() => setTerm(null)} />;

  return (
    <div className="app">
      <Sidebar
        view={view}
        setView={setView}
        collapsed={collapsed}
        toggleCollapse={toggleCollapse}
        theme={theme}
        toggleTheme={() => setTheme(theme === "dark" ? "light" : "dark")}
        user={me?.user || ""}
        isAdmin={!!me?.is_admin}
        local={isTauri}
        onLogout={logout}
        onDocs={() => openDoc("")}
        ghLogin={gh.login}
        onGithubLogin={openGithubLogin}
      />
      <main className="main">
        {vaultLegacy && (
          <div className="vault-migrate">
            <span>检测到旧密码保险库。已改用本机设备密钥自动解锁 —— 需迁移一次（旧凭据会清空，之后重新录入）。</span>
            <button disabled={migrating} onClick={migrateVault}>{migrating ? "迁移中…" : "迁移保险库"}</button>
          </div>
        )}
        {view === "home" && <Home teamPath={teamPath} servers={me?.servers ?? []} onOpen={openInWorkspace} />}
        {/* 桌面工作区：常驻挂载，切走只 CSS 隐藏（卸载 = 关掉所有活会话）。web 走门户 Workspaces。 */}
        {isTauri && (
          <div className={"ws-host" + (view === "workspaces" ? "" : " hidden")}>
            <Workspace ref={wsRef} empty="去「主页」的织物图点一个节点，这里开出它的终端。" />
          </div>
        )}
        {!isTauri && view === "workspaces" && <Workspaces goSettings={() => setView("settings")} />}
        {view === "servers" && (
          <Servers
            me={me}
            reload={reload}
            goSettings={() => setView("settings")}
            goTerminal={goTerminal}
            teamPath={teamPath}
            goTeam={() => setView("team")}
          />
        )}
        {view === "team" && (
          <Team
            reload={reload}
            goServers={() => setView("servers")}
            goSettings={() => setView("settings")}
            teamPath={teamPath}
            setTeamPath={setTeamPath}
            onReauth={logout}
          />
        )}
        {view === "settings" && <Settings me={me} reload={reload} theme={theme} setTheme={setTheme} />}
        {view === "admin" && me?.is_admin && <Admin me={me} />}
        {view === "docs" && (
          <Docs
            tree={docTree}
            active={activeDoc}
            openDoc={openDoc}
            slugs={collectSlugs(docTree)}
            childDocs={groupChildren(docTree, activeDoc)}
            crumbs={breadcrumb(docTree, activeDoc)}
            pager={neighbors(docTree, activeDoc)}
          />
        )}
      </main>
    </div>
  );
}
