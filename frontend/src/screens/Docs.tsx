import { marked } from "marked";
import { useEffect, useRef, useState } from "react";

import { api } from "../api";

marked.setOptions({ gfm: true, breaks: false });

const EMPTY = '<div class="ws-empty">还没有文档 · 在服务器 <code>~/gateway/portal/docs/</code> 放置 .md 文件即可</div>';

interface Head { id: string; text: string; level: number; }

function slug(s: string, i: number) {
  const base = s.toLowerCase().trim().replace(/[^\w一-龥]+/g, "-").replace(/^-+|-+$/g, "");
  return (base || "sec") + "-" + i;
}

export function Docs({ hasDocs, active }: { hasDocs: boolean; active: string }) {
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

  // 正文渲染后：给标题打 id，抽取「本页目录」。
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
    setHeads(list);
    setCurHead(list[0]?.id || "");
  }, [html]);

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

  return (
    <div className="docs-wrap">
      <article className="docs-main">
        <div className="md-body" ref={bodyRef} dangerouslySetInnerHTML={{ __html: html }} />
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
