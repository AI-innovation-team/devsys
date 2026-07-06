import { useCallback, useEffect, useState } from "react";

import { api, collectSlugs, DocNode, firstDocSlug, Me } from "./api";
import { Sidebar } from "./components/Sidebar";
import { Docs } from "./screens/Docs";
import { Servers } from "./screens/Servers";
import { Settings } from "./screens/Settings";
import { Workspaces } from "./screens/Workspaces";

export type View = "workspaces" | "servers" | "docs" | "settings";
export type Theme = "light" | "dark";

const VIEWS: View[] = ["workspaces", "servers", "docs", "settings"];

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
  const [collapsed, setCollapsed] = useState<boolean>(() => {
    try { return localStorage.getItem("devsys.rail") === "1"; } catch { return false; }
  });
  const [theme, setThemeState] = useState<Theme>(
    () => (document.documentElement.getAttribute("data-theme") === "dark" ? "dark" : "light"),
  );

  // 文档：层级树 + 当前项 + 侧栏飞出目录开关（状态提到这里，供侧栏渲染飞出面板）
  const [docTree, setDocTree] = useState<DocNode[]>([]);
  const [activeDoc, setActiveDoc] = useState("");
  const [docsNav, setDocsNav] = useState(false);

  const reload = useCallback(async () => {
    try { setMe(await api.me()); } catch { /* Caddy 网关外层已鉴权 */ }
  }, []);
  useEffect(() => { reload(); }, [reload]);

  useEffect(() => {
    (async () => {
      try {
        const d = await api.docs();
        setDocTree(d.tree);
        const slugs = collectSlugs(d.tree);
        if (!slugs.length) return;
        let last = "";
        try { last = localStorage.getItem("devsys.doc") || ""; } catch { /* ignore */ }
        setActiveDoc(slugs.includes(last) ? last : firstDocSlug(d.tree));
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
  const toggleCollapse = () => {
    setCollapsed((c) => {
      const n = !c;
      try { localStorage.setItem("devsys.rail", n ? "1" : "0"); } catch {}
      return n;
    });
  };

  // 侧栏「文档」按钮：进入文档视图并开合飞出目录
  const onDocsNav = () => {
    if (view === "docs") setDocsNav((o) => !o);
    else { setView("docs"); setDocsNav(true); }
  };
  const openDoc = (slug: string) => {
    setActiveDoc(slug);
    setDocsNav(false);
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
        docTree={docTree}
        activeDoc={activeDoc}
        openDoc={openDoc}
        onDocsNav={onDocsNav}
        docsNav={docsNav}
        setDocsNav={setDocsNav}
      />
      <main className="main">
        {view === "workspaces" && <Workspaces goSettings={() => setView("settings")} />}
        {view === "servers" && <Servers me={me} goSettings={() => setView("settings")} />}
        {view === "settings" && <Settings me={me} reload={reload} theme={theme} setTheme={setTheme} />}
        {view === "docs" && <Docs hasDocs={docTree.length > 0} active={activeDoc} />}
      </main>
    </div>
  );
}
