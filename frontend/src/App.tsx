import { useCallback, useEffect, useState } from "react";

import { api, breadcrumb, collectSlugs, DocNode, groupChildren, neighbors, Me } from "./api";
import { Sidebar } from "./components/Sidebar";
import { Admin } from "./screens/Admin";
import { Docs } from "./screens/Docs";
import { Servers } from "./screens/Servers";
import { Settings } from "./screens/Settings";
import { Workspaces } from "./screens/Workspaces";

export type View = "workspaces" | "servers" | "docs" | "settings" | "admin";
export type Theme = "light" | "dark";

const VIEWS: View[] = ["workspaces", "servers", "docs", "settings", "admin"];

function restoreView(): View {
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

  const reload = useCallback(async () => {
    try { setMe(await api.me()); } catch { /* Caddy 网关外层已鉴权 */ }
  }, []);
  useEffect(() => { reload(); }, [reload]);

  useEffect(() => {
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
        onDocs={() => openDoc("")}
      />
      <main className="main">
        {view === "workspaces" && <Workspaces goSettings={() => setView("settings")} />}
        {view === "servers" && <Servers me={me} goSettings={() => setView("settings")} />}
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
