import { useCallback, useEffect, useState } from "react";

import { api, Me } from "./api";
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

  const reload = useCallback(async () => {
    try { setMe(await api.me()); } catch { /* Caddy 网关外层已鉴权 */ }
  }, []);
  useEffect(() => { reload(); }, [reload]);

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
      />
      <main className="main">
        {view === "workspaces" && <Workspaces goSettings={() => setView("settings")} />}
        {view === "servers" && <Servers me={me} goSettings={() => setView("settings")} />}
        {view === "settings" && <Settings me={me} reload={reload} theme={theme} setTheme={setTheme} />}
        {view === "docs" && <Docs />}
      </main>
    </div>
  );
}
