import { marked } from "marked";
import { useEffect, useState } from "react";

import { api, DocMeta } from "../api";

marked.setOptions({ gfm: true, breaks: false });

const EMPTY = '<div class="ws-empty">还没有文档 · 在服务器 <code>~/gateway/portal/docs/</code> 放置 .md 文件即可</div>';

export function Docs() {
  const [docs, setDocs] = useState<DocMeta[]>([]);
  const [active, setActive] = useState("");
  const [html, setHtml] = useState('<div class="ws-empty">加载中…</div>');

  useEffect(() => {
    (async () => {
      try {
        const d = await api.docs();
        setDocs(d.docs);
        if (!d.docs.length) { setHtml(EMPTY); return; }
        let last = "";
        try { last = localStorage.getItem("devsys.doc") || ""; } catch { /* ignore */ }
        open(d.docs.some((x) => x.slug === last) ? last : d.docs[0].slug);
      } catch {
        setHtml('<div class="ws-empty err">文档加载失败</div>');
      }
    })();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const open = async (slug: string) => {
    setActive(slug);
    try { localStorage.setItem("devsys.doc", slug); } catch { /* ignore */ }
    try {
      const d = await api.doc(slug);
      setHtml(marked.parse(d.content) as string);
    } catch {
      setHtml('<div class="ws-empty err">无法打开文档</div>');
    }
  };

  return (
    <div className="docs-wrap">
      <aside className="docs-side">
        <div className="eyebrow">docs</div>
        <div className="docs-list">
          {docs.map((x) => (
            <button key={x.slug} className={"docs-item" + (active === x.slug ? " active" : "")} onClick={() => open(x.slug)}>
              {x.title}
            </button>
          ))}
        </div>
      </aside>
      <article className="docs-main">
        <div className="md-body" dangerouslySetInnerHTML={{ __html: html }} />
      </article>
    </div>
  );
}
