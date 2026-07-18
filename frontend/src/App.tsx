import { useCallback, useEffect, useState } from "react";

import { api, breadcrumb, collectSlugs, DocNode, groupChildren, neighbors, Me } from "./api";
import { data } from "./data";
import { isTauri } from "./transport";
import { AuthGate } from "./components/AuthGate";
import { Sidebar } from "./components/Sidebar";
import { Admin } from "./screens/Admin";
import { Docs } from "./screens/Docs";
import { Servers } from "./screens/Servers";
import { Settings } from "./screens/Settings";
import { Team } from "./screens/Team";
import { Terminal } from "./screens/Terminal";
import { Home } from "./screens/Home";
import { Workspaces } from "./screens/Workspaces";

export type View = "home" | "workspaces" | "servers" | "team" | "docs" | "settings" | "admin";
export type Theme = "light" | "dark";

const VIEWS: View[] = ["home", "workspaces", "servers", "team", "docs", "settings", "admin"];

// 自包含 app（tauri）无门户的工作区/文档/管理，默认落在「主页」驾驶舱。
const LOCAL_VIEWS: View[] = ["home", "servers", "team", "settings"];

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

  // 保险库在登录门已解锁，会话内无需再次解锁。
  const goTerminal = (server: string, ws = "") => setTerm({ server, ws });

  // 本地退出登录：锁库 + 回到登录门。
  const logout = async () => {
    try { await data.vaultLock(); } catch { /* ignore */ }
    setTerm(null);
    setMe(null);
    setGate({ exists: true });
  };

  // 本地登录门（tauri）：未解锁前不进 app。web 无门（门户负责鉴权）。
  const [gate, setGate] = useState<"checking" | { exists: boolean } | null>(isTauri ? "checking" : null);
  useEffect(() => {
    if (!isTauri) return;
    data.vaultState()
      .then((st) => setGate(st.unlocked ? null : { exists: st.exists }))
      .catch(() => setGate({ exists: false }));
  }, []);

  const reload = useCallback(async () => {
    try { setMe(await data.loadMe()); } catch { /* web: Caddy 外层已鉴权；tauri: 本地空清单 */ }
  }, []);
  useEffect(() => { if (!isTauri || gate === null) reload(); }, [reload, gate]);

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
  if (gate) return <AuthGate exists={gate.exists} onDone={() => setGate(null)} onReset={() => setGate({ exists: false })} />;

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
      />
      <main className="main">
        {view === "home" && <Home teamPath={teamPath} servers={me?.servers ?? []} />}
        {view === "workspaces" && <Workspaces goSettings={() => setView("settings")} />}
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
