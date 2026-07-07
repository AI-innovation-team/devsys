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
  onDocs: () => void; // 「文档」按钮：进入文档首页
}

const NAV: { id: View; icon: string; label: string }[] = [
  { id: "workspaces", icon: "grid", label: "工作区" },
  { id: "servers", icon: "terminal", label: "服务器" },
  { id: "docs", icon: "file", label: "文档" },
];

export function Sidebar({ view, setView, collapsed, toggleCollapse, theme, toggleTheme, user, onDocs }: Props) {
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
          <span className="brand-tile"><BrandMark /></span>
          <div className="brand-name">AIT.dev</div>
        </div>
      </div>

      <div className="nav">
        <div className="nav-sec">工作台</div>
        {NAV.map((n) => (
          <button
            key={n.id}
            className={"nav-item" + (view === n.id ? " active" : "")}
            onClick={() => (n.id === "docs" ? onDocs() : setView(n.id))}
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
          </div>
          <Icon name="updown" className="avatar-chev" />
        </button>
        {menuOpen && (
          <div className="menu">
            <button onClick={() => { setView("settings"); setMenuOpen(false); }}>
              <Icon name="settings" />设置
            </button>
            <button onClick={toggleTheme}>
              <Icon name={theme === "dark" ? "sun" : "moon"} />{theme === "dark" ? "浅色模式" : "深色模式"}
            </button>
            <div className="menu-sep" />
            <a href="/oauth2/sign_out?rd=/oauth2/sign_in" className="danger">
              <Icon name="logout" />退出登录
            </a>
          </div>
        )}
      </div>
    </nav>
  );
}

// 品牌标志：等距立方体，顶面用赤陶色点缀，象征“系统 / 基础设施”。
function BrandMark() {
  return (
    <svg className="brand-mark" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
      <path d="M12 2.6 20.5 7.3 12 12 3.5 7.3Z" fill="var(--accent)" />
      <path d="M3.5 7.9 11.4 12.3 11.4 21.4 3.5 17Z" fill="currentColor" fillOpacity="0.9" />
      <path d="M20.5 7.9 12.6 12.3 12.6 21.4 20.5 17Z" fill="currentColor" fillOpacity="0.55" />
    </svg>
  );
}
