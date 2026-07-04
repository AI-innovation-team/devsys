import { useEffect, useRef, useState } from "react";

import { Theme, View } from "../App";
import { Icon } from "../icons";

interface Props {
  view: View;
  setView: (v: View) => void;
  collapsed: boolean;
  toggleCollapse: () => void;
  theme: Theme;
  toggleTheme: () => void;
  user: string;
}

const NAV: { id: View; icon: string; label: string }[] = [
  { id: "workspaces", icon: "grid", label: "工作区" },
  { id: "servers", icon: "terminal", label: "服务器" },
  { id: "docs", icon: "file", label: "文档" },
];

export function Sidebar({ view, setView, collapsed, toggleCollapse, theme, toggleTheme, user }: Props) {
  const [menuOpen, setMenuOpen] = useState(false);
  const footRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (footRef.current && !footRef.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener("click", onDoc);
    return () => document.removeEventListener("click", onDoc);
  }, [menuOpen]);

  return (
    <nav className={"rail" + (collapsed ? " collapsed" : "")}>
      <div className="rail-top">
        <button className="rail-toggle" onClick={toggleCollapse} title="收起 / 展开" aria-label="收起或展开侧栏">
          <Icon name="panel" />
        </button>
        <div className="brand">
          <span className="brand-tile"><Icon name="terminal" /></span>
          <div>
            <div className="brand-name">devsys</div>
            <div className="brand-sub">内网开发者门户</div>
          </div>
        </div>
      </div>

      <div className="nav">
        <div className="nav-sec">工作台</div>
        {NAV.map((n) => (
          <button
            key={n.id}
            className={"nav-item" + (view === n.id ? " active" : "")}
            onClick={() => setView(n.id)}
          >
            <Icon name={n.icon} />
            <span className="lbl">{n.label}</span>
          </button>
        ))}
      </div>

      <div className="rail-bottom">
        <button className="nav-item" onClick={toggleTheme}>
          <Icon name={theme === "dark" ? "sun" : "moon"} />
          <span className="lbl">{theme === "dark" ? "浅色模式" : "深色模式"}</span>
        </button>
      </div>

      <div className="rail-foot" ref={footRef}>
        <button className="avatar-btn" onClick={(e) => { e.stopPropagation(); setMenuOpen((o) => !o); }} aria-label="用户菜单">
          <span className="avatar">{(user || "·").slice(0, 1)}</span>
          <div className="avatar-meta">
            <div className="avatar-name">{user || "…"}</div>
            <div className="avatar-sub">GitHub 身份</div>
          </div>
          <Icon name="updown" className="avatar-chev" />
        </button>
        {menuOpen && (
          <div className="menu">
            <button onClick={() => { setView("settings"); setMenuOpen(false); }}>
              <Icon name="settings" />设置
            </button>
            <div className="menu-sep" />
            <a href="/oauth2/sign_out" className="danger">
              <Icon name="logout" />退出登录
            </a>
          </div>
        )}
      </div>
    </nav>
  );
}
