import { marked } from "marked";
import { MouseEvent as ReactMouseEvent, useEffect, useRef, useState } from "react";

import { api, Crumb, DocNode, firstDocSlug, PageRef } from "../api";
import { Icon } from "../icons";

marked.setOptions({ gfm: true, breaks: false });

const EMPTY = '<div class="ws-empty">还没有文档 · 在服务器 <code>~/gateway/docs/</code> 放置 .md 文件即可</div>';

interface Head { id: string; text: string; level: number; }

function slug(s: string, i: number) {
  const base = s.toLowerCase().trim().replace(/[^\w一-龥]+/g, "-").replace(/^-+|-+$/g, "");
  return (base || "sec") + "-" + i;
}

/** 把正文里的 markdown 链接解析成有效 doc slug；支持相对（同目录/../）与从根起的绝对写法。返回 null 表示非内部链接。 */
function resolveDocLink(href: string, from: string, valid: string[]): string | null {
  let h = href.split("#")[0].split("?")[0].replace(/\.md$/i, "");
  if (!h) return null;
  // 1) 直接当作从 docs 根起的绝对 slug
  const abs = h.replace(/^\/+/, "");
  if (valid.includes(abs)) return abs;
  // 2) 相对当前文档所在目录解析
  const dir = from.includes("/") ? from.slice(0, from.lastIndexOf("/")) : "";
  const parts = dir ? dir.split("/") : [];
  for (const seg of h.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") parts.pop();
    else parts.push(seg);
  }
  const rel = parts.join("/");
  return valid.includes(rel) ? rel : null;
}

interface DocsProps {
  tree: DocNode[];
  active: string; // 空串 = 文档首页
  openDoc: (slug: string) => void;
  slugs: string[];
  childDocs: DocNode[]; // 若当前是分组着陆页，则为其子节点（用于自动生成「本节文档」链接）
  crumbs: Crumb[]; // 面包屑：根 → 各级分组 → 当前页
  pager: { prev: PageRef | null; next: PageRef | null }; // 线性上一/下一页
}

/** 子节点的打开目标：doc 用自身 slug；group 用其 _index，没有则用第一篇后代文档。 */
function childTarget(n: DocNode): string {
  if (n.type === "doc") return n.slug;
  return n.slug || firstDocSlug(n.children);
}

/** 分组下可打开页的数量（含 _index 与所有后代文档）。 */
function countPages(n: DocNode): number {
  if (n.type === "doc") return 1;
  return (n.slug ? 1 : 0) + n.children.reduce((s, c) => s + countPages(c), 0);
}

export function Docs({ tree, active, openDoc, slugs, childDocs, crumbs, pager }: DocsProps) {
  const hasDocs = tree.length > 0;
  const [html, setHtml] = useState('<div class="ws-empty">加载中…</div>');
  const [heads, setHeads] = useState<Head[]>([]);
  const [curHead, setCurHead] = useState("");
  const bodyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!hasDocs) { setHtml(EMPTY); return; }
    if (!active) return;
    (async () => {
      try {
        const d = await api.doc(active);
        setHtml(marked.parse(d.content) as string);
      } catch {
        setHtml('<div class="ws-empty err">无法打开文档</div>');
      }
    })();
  }, [active, hasDocs]);

  // 正文渲染后：给标题打 id、抽取「本页目录」，并让外链在新页打开。
  useEffect(() => {
    const el = bodyRef.current;
    if (!el) { setHeads([]); return; }
    const list: Head[] = [];
    (Array.from(el.querySelectorAll("h1, h2, h3")) as HTMLElement[]).forEach((h, i) => {
      const level = Number(h.tagName[1]);
      const id = slug(h.textContent || "", i);
      h.id = id;
      if (level > 1) list.push({ id, text: h.textContent || "", level });
    });
    (Array.from(el.querySelectorAll("a[href]")) as HTMLAnchorElement[]).forEach((a) => {
      const href = a.getAttribute("href") || "";
      if (/^[a-z]+:/i.test(href) || href.startsWith("//")) {
        a.target = "_blank";
        a.rel = "noopener noreferrer";
      }
    });
    setHeads(list);
    setCurHead(list[0]?.id || "");
    el.closest(".main")?.scrollTo({ top: 0 });
  }, [html]);

  // 拦截正文内的站内文档链接：解析成 slug 走 openDoc，避免整页跳转/回到自身。
  const onBodyClick = (e: ReactMouseEvent) => {
    const a = (e.target as HTMLElement).closest("a");
    if (!a) return;
    const href = a.getAttribute("href") || "";
    if (!href || href.startsWith("#")) return; // 锚点：保持默认滚动
    if (/^[a-z]+:/i.test(href) || href.startsWith("//")) return; // 外链：新页打开
    e.preventDefault(); // 相对/绝对内部链接一律不整页跳转
    const s = resolveDocLink(href, active, slugs);
    if (s) openDoc(s);
  };

  // 滚动高亮当前小节。
  useEffect(() => {
    const el = bodyRef.current;
    if (!el || !heads.length) return;
    const io = new IntersectionObserver(
      (ents) => {
        const vis = ents.filter((e) => e.isIntersecting).sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
        if (vis[0]) setCurHead(vis[0].target.id);
      },
      { rootMargin: "-8% 0px -80% 0px", threshold: 0 },
    );
    heads.forEach((h) => { const n = document.getElementById(h.id); if (n) io.observe(n); });
    return () => io.disconnect();
  }, [heads]);

  const jump = (id: string) => {
    document.getElementById(id)?.scrollIntoView({ behavior: "smooth", block: "start" });
    setCurHead(id);
  };

  // 文档首页：根据目录树自动生成导览
  if (active === "") {
    const loose = tree.filter((n) => n.type === "doc");
    const groups = tree.filter((n) => n.type === "group");
    const card = (n: DocNode) => {
      const t = childTarget(n);
      if (!t) return null;
      return (
        <button key={n.type === "group" ? n.path : n.slug} className="home-card" onClick={() => openDoc(t)}>
          <span className="home-card-t">{n.title}</span>
          {n.type === "group" && <span className="home-card-sub">{countPages(n)} 篇</span>}
          <Icon name="chevron" className="home-card-arw" />
        </button>
      );
    };
    return (
      <div className="docs-wrap">
        <article className="docs-main doc-home">
          <h1 className="home-title">文档</h1>
          <p className="home-sub">AIT.dev 内网门户的使用与运维文档。选择一个主题开始。</p>
          {!hasDocs && <div className="ws-empty">还没有文档 · 在服务器的 docs 目录放置 .md 文件即可</div>}
          {loose.length > 0 && <div className="home-grid">{loose.map(card)}</div>}
          {groups.map((g) => (
            <section className="home-sec" key={g.type === "group" ? g.path : g.title}>
              <h2 className="home-sec-h">
                {g.type === "group" && g.slug
                  ? <button className="home-sec-link" onClick={() => openDoc(g.slug!)}>{g.title}</button>
                  : <span>{g.title}</span>}
              </h2>
              {g.type === "group" && <div className="home-grid">{g.children.map(card)}</div>}
            </section>
          ))}
        </article>
      </div>
    );
  }

  return (
    <div className="docs-wrap">
      <article className="docs-main">
        {hasDocs && crumbs.length > 0 && (
          <nav className="crumbs" aria-label="位置">
            <button className="crumb-root link" onClick={() => openDoc("")}>文档</button>
            {crumbs.map((c, i) => {
              const last = i === crumbs.length - 1;
              return (
                <span key={c.slug || c.title + i} className="crumb-seg">
                  <Icon name="chevron" className="crumb-sep" />
                  {last || !c.slug ? (
                    <span className={"crumb" + (last ? " cur" : "")}>{c.title}</span>
                  ) : (
                    <button className="crumb link" onClick={() => openDoc(c.slug!)}>{c.title}</button>
                  )}
                </span>
              );
            })}
          </nav>
        )}

        <div className="md-body" ref={bodyRef} onClick={onBodyClick} dangerouslySetInnerHTML={{ __html: html }} />

        {childDocs.length > 0 && (
          <nav className="doc-children">
            <div className="doc-children-h">本节文档</div>
            <div className="doc-children-list">
              {childDocs.map((n) => {
                const t = childTarget(n);
                if (!t) return null;
                return (
                  <button key={n.type === "group" ? n.path : n.slug} className="doc-child" onClick={() => openDoc(t)}>
                    <Icon name="file" className="doc-child-ic" />
                    <span className="doc-child-title">{n.title}</span>
                    <Icon name="chevron" className="doc-child-arw" />
                  </button>
                );
              })}
            </div>
          </nav>
        )}

        {hasDocs && (pager.prev || pager.next) && (
          <nav className="doc-pager" aria-label="翻页">
            {pager.prev ? (
              <button className="pager-btn prev" onClick={() => openDoc(pager.prev!.slug)}>
                <Icon name="chevron" className="pager-arw" />
                <span className="pager-meta"><span className="pager-lbl">上一篇</span><span className="pager-t">{pager.prev.title}</span></span>
              </button>
            ) : <span />}
            {pager.next ? (
              <button className="pager-btn next" onClick={() => openDoc(pager.next!.slug)}>
                <span className="pager-meta"><span className="pager-lbl">下一篇</span><span className="pager-t">{pager.next.title}</span></span>
                <Icon name="chevron" className="pager-arw" />
              </button>
            ) : <span />}
          </nav>
        )}
      </article>

      {heads.length > 1 && (
        <aside className="docs-toc">
          <div className="docs-toc-title">本页目录</div>
          <nav className="docs-toc-list">
            {heads.map((h) => (
              <button
                key={h.id}
                className={"toc-link lv" + h.level + (curHead === h.id ? " on" : "")}
                onClick={() => jump(h.id)}
              >
                {h.text}
              </button>
            ))}
          </nav>
        </aside>
      )}
    </div>
  );
}
