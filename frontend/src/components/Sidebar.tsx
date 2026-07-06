import { useEffect, useRef, useState } from "react";

import { Theme, View } from "../App";
import { DocNode } from "../api";
import { Icon } from "../icons";

interface Props {
  view: View;
  setView: (v: View) => void;
  collapsed: boolean;
  toggleCollapse: () => void;
  theme: Theme;
  toggleTheme: () => void;
  user: string;
  docTree: DocNode[];
  activeDoc: string;
  openDoc: (slug: string) => void;
  onDocsNav: () => void;
  docsNav: boolean;
  setDocsNav: (v: boolean) => void;
}

/** slug 的各级祖先分组路径，如 a/b/c.md → ["a", "a/b"]。 */
function ancestorGroups(slug: string): string[] {
  const parts = slug.split("/");
  parts.pop();
  const out: string[] = [];
  let cur = "";
  for (const p of parts) { cur = cur ? cur + "/" + p : p; out.push(cur); }
  return out;
}

const NAV: { id: View; icon: string; label: string }[] = [
  { id: "workspaces", icon: "grid", label: "工作区" },
  { id: "servers", icon: "terminal", label: "服务器" },
  { id: "docs", icon: "file", label: "文档" },
];

export function Sidebar({ view, setView, collapsed, toggleCollapse, theme, toggleTheme, user, docTree, activeDoc, openDoc, onDocsNav, docsNav, setDocsNav }: Props) {
  const [menuOpen, setMenuOpen] = useState(false);
  const footRef = useRef<HTMLDivElement>(null);
  const docsBtnRef = useRef<HTMLButtonElement>(null);
  const flyRef = useRef<HTMLDivElement>(null);
  const [flyPos, setFlyPos] = useState<{ top: number; left: number }>({ top: 0, left: 0 });
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  // 当前文档所在分组自动展开
  useEffect(() => {
    if (!activeDoc) return;
    const anc = ancestorGroups(activeDoc);
    if (anc.length) setExpanded((prev) => new Set([...prev, ...anc]));
  }, [activeDoc]);
  const toggleGroup = (path: string) => setExpanded((prev) => {
    const n = new Set(prev);
    n.has(path) ? n.delete(path) : n.add(path);
    return n;
  });

  useEffect(() => {
    if (!menuOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (footRef.current && !footRef.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener("click", onDoc);
    return () => document.removeEventListener("click", onDoc);
  }, [menuOpen]);

  // 文档飞出：定位到「文档」按钮旁，点外部 / Esc 收起
  useEffect(() => {
    if (!docsNav) return;
    const place = () => {
      const r = docsBtnRef.current?.getBoundingClientRect();
      if (!r) return;
      const mobile = window.innerWidth <= 760;
      setFlyPos(mobile ? { top: r.bottom + 6, left: Math.max(8, r.left) } : { top: r.top, left: r.right + 8 });
    };
    place();
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (docsBtnRef.current?.contains(t) || flyRef.current?.contains(t)) return;
      setDocsNav(false);
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setDocsNav(false); };
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [docsNav, setDocsNav]);

  return (
    <nav className={"rail" + (collapsed ? " collapsed" : "")}>
      <div className="rail-top">
        <button className="rail-toggle" onClick={toggleCollapse} title="收起 / 展开" aria-label="收起或展开侧栏">
          <Icon name="panel" />
        </button>
        <div className="brand">
          <span className="brand-tile"><BrandMark /></span>
          <div className="brand-name">devsys</div>
        </div>
      </div>

      <div className="nav">
        <div className="nav-sec">工作台</div>
        {NAV.map((n) => (
          n.id === "docs" ? (
            <button
              key={n.id}
              ref={docsBtnRef}
              className={"nav-item" + (view === n.id ? " active" : "") + (docsNav ? " flyon" : "")}
              onClick={onDocsNav}
              aria-expanded={docsNav}
            >
              <Icon name={n.icon} />
              <span className="lbl">{n.label}</span>
              <Icon name="chevron" className="nav-caret" />
            </button>
          ) : (
            <button
              key={n.id}
              className={"nav-item" + (view === n.id ? " active" : "")}
              onClick={() => setView(n.id)}
            >
              <Icon name={n.icon} />
              <span className="lbl">{n.label}</span>
            </button>
          )
        ))}
      </div>

      {docsNav && (
        <div className="docs-fly" ref={flyRef} style={{ top: flyPos.top, left: flyPos.left }}>
          <div className="docs-fly-title">文档</div>
          {docTree.length ? (
            <div className="docs-list">
              <DocTree nodes={docTree} depth={0} activeDoc={activeDoc} openDoc={openDoc} expanded={expanded} toggle={toggleGroup} />
            </div>
          ) : (
            <div className="docs-fly-empty">暂无文档</div>
          )}
        </div>
      )}

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
            <a href="/oauth2/sign_out" className="danger">
              <Icon name="logout" />退出登录
            </a>
          </div>
        )}
      </div>
    </nav>
  );
}

// 递归渲染文档层级树：doc → 可点条目；group → 可折叠分组（有 _index 则标题可打开着陆页）。
interface TreeProps {
  nodes: DocNode[];
  depth: number;
  activeDoc: string;
  openDoc: (slug: string) => void;
  expanded: Set<string>;
  toggle: (path: string) => void;
}
function DocTree({ nodes, depth, activeDoc, openDoc, expanded, toggle }: TreeProps) {
  return (
    <>
      {nodes.map((n) => {
        if (n.type === "doc") {
          return (
            <button
              key={n.slug}
              className={"docs-item" + (activeDoc === n.slug ? " active" : "")}
              style={{ paddingLeft: 12 + depth * 14 }}
              onClick={() => openDoc(n.slug)}
            >
              {n.title}
            </button>
          );
        }
        const open = expanded.has(n.path);
        const hasKids = n.children.length > 0;
        const isActive = !!n.slug && activeDoc === n.slug;
        return (
          <div key={n.path} className="docs-group-wrap">
            <div className={"docs-group" + (isActive ? " active" : "")} style={{ paddingLeft: 4 + depth * 14 }}>
              <button
                className="grp-caret-btn"
                onClick={() => toggle(n.path)}
                aria-label={open ? "折叠分组" : "展开分组"}
                style={{ visibility: hasKids ? "visible" : "hidden" }}
              >
                <Icon name="chevron" className={"grp-caret" + (open ? " open" : "")} />
              </button>
              <button className="grp-label" onClick={() => (n.slug ? openDoc(n.slug) : toggle(n.path))}>
                {n.title}
              </button>
            </div>
            {open && hasKids && (
              <DocTree nodes={n.children} depth={depth + 1} activeDoc={activeDoc} openDoc={openDoc} expanded={expanded} toggle={toggle} />
            )}
          </div>
        );
      })}
    </>
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
