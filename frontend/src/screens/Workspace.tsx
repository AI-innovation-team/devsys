import { useCallback, useEffect, useImperativeHandle, useRef, useState, forwardRef } from "react";

import { TermView } from "../components/TermView";
import { LOCAL_NODE } from "../data";
import { Icon } from "../icons";

// 工作区 = 驾驶舱的「工作面」：多个活终端 pane，标签切换 + 可分屏。
// 每个 pane 一条独立 SSH 会话（后端 Sessions 按 id 支持 N 并发）。
// 关键：非活动 pane **保持挂载**、只用 CSS 隐藏 —— 卸载会关掉会话、丢掉正在跑的活。
//
// 持久化（阶段 E）：每个 pane 带一个稳定的 `ws` 名，后端据此跑 `tmux new-session -A`
// （attach-or-create）。于是关掉 app / 断网，远端的活照跑；下次开同名 ws 就接回原样。
// 布局（开了哪些 pane）落 localStorage，启动时重开 → 重开即重连即接回。
export interface Pane {
  id: string;
  server: string;
  ws: string;
  title: string;
  conn: boolean | null;
}

export interface WorkspaceHandle {
  openPane: (server: string, opts?: { fresh?: boolean; ws?: string }) => void;
}

const LS_KEY = "devsys.wsp";

let seq = 1;

// 服务器名 → 可进 tmux 会话名的 slug。后端只收 [A-Za-z0-9_-]（注入防线），这里先对齐，
// 免得等到连接时才报错。
const slug = (s: string) => s.replace(/[^A-Za-z0-9_-]/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "node";

// 给这台机取一个没被占用的持久工作区名。同机开第二个终端 → -2、-3……
function autoWs(server: string, taken: Set<string>): string {
  const base = `ait-${slug(server)}`;
  for (let i = 1; ; i++) {
    const n = `${base}-${i}`;
    if (!taken.has(n)) return n;
  }
}

type Saved = { panes: { server: string; ws: string }[]; active: number; split: boolean };

function restore(): Saved {
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (!raw) return { panes: [], active: 0, split: false };
    const s = JSON.parse(raw) as Saved;
    if (!Array.isArray(s?.panes)) return { panes: [], active: 0, split: false };
    return {
      panes: s.panes.filter((p) => p && typeof p.server === "string" && typeof p.ws === "string"),
      active: typeof s.active === "number" ? s.active : 0,
      split: !!s.split,
    };
  } catch {
    return { panes: [], active: 0, split: false };
  }
}

export const Workspace = forwardRef<WorkspaceHandle, { empty?: React.ReactNode }>(function Workspace(
  { empty },
  ref,
) {
  const saved = useRef<Saved>(restore()).current;
  const [panes, setPanes] = useState<Pane[]>(() =>
    saved.panes.map((p) => ({
      id: `pane-${seq++}`,
      server: p.server,
      ws: p.ws,
      title: p.server,
      conn: null,
    })),
  );
  const [active, setActive] = useState<string>("");
  const [split, setSplit] = useState(saved.split);
  const activeRef = useRef(active);
  activeRef.current = active;

  // 恢复出来的 pane 里挑回上次的活动标签（按下标，id 每次都是新的）。
  useEffect(() => {
    if (!active && panes.length) {
      const i = Math.min(Math.max(saved.active, 0), panes.length - 1);
      setActive(panes[i].id);
    }
    // 只在首帧补一次活动标签。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 布局落盘：下次启动重开同样这些 (server, ws) → tmux 接回原样。
  useEffect(() => {
    try {
      const i = panes.findIndex((p) => p.id === active);
      const s: Saved = {
        panes: panes.map((p) => ({ server: p.server, ws: p.ws })),
        active: i < 0 ? 0 : i,
        split,
      };
      localStorage.setItem(LS_KEY, JSON.stringify(s));
    } catch {
      /* ignore */
    }
  }, [panes, active, split]);

  const openPane = useCallback((server: string, opts?: { fresh?: boolean; ws?: string }) => {
    setPanes((cur) => {
      // 同一台机已开着 → 只切过去，不重复开会话（除非明确要「再开一个」）。
      if (!opts?.fresh && !opts?.ws) {
        const hit = cur.find((p) => p.server === server);
        if (hit) { setActive(hit.id); return cur; }
      }
      const ws = opts?.ws ?? autoWs(server, new Set(cur.map((p) => p.ws)));
      const hit = cur.find((p) => p.server === server && p.ws === ws);
      if (hit) { setActive(hit.id); return cur; }
      const id = `pane-${seq++}`;
      setActive(id);
      return [...cur, { id, server, ws, title: server, conn: null }];
    });
  }, []);

  useImperativeHandle(ref, () => ({ openPane }), [openPane]);

  // 关标签 = 断开这条 SSH，但远端 tmux 里的活**继续跑**；再开同机就接回来。
  const closePane = useCallback((id: string) => {
    setPanes((cur) => {
      const next = cur.filter((p) => p.id !== id);
      if (activeRef.current === id) setActive(next.length ? next[next.length - 1].id : "");
      if (next.length < 2) setSplit(false);
      return next;
    });
  }, []);

  const patch = useCallback((id: string, d: Partial<Pane>) => {
    setPanes((cur) => cur.map((p) => (p.id === id ? { ...p, ...d } : p)));
  }, []);

  // 分屏时可见的 pane：活动的 + 它后面那个（2-up）。否则只有活动的。
  const idx = panes.findIndex((p) => p.id === active);
  const visible = new Set<string>();
  if (idx >= 0) {
    visible.add(panes[idx].id);
    if (split && panes.length > 1) visible.add(panes[(idx + 1) % panes.length].id);
  }
  const cur = idx >= 0 ? panes[idx] : null;
  // 同一台机开了多个 pane 时，标签得能分辨 —— 用工作区名尾巴的序号（ait-turing-2 → 2）。
  const dup = new Set(panes.filter((p, i) => panes.findIndex((q) => q.server === p.server) !== i).map((p) => p.server));
  const nick = (s: string) => (s === LOCAL_NODE ? "本机" : s);
  const tabLabel = (p: Pane) => (dup.has(p.server) ? `${nick(p.server)} ${p.ws.split("-").pop()}` : nick(p.server));

  return (
    <div className="wsp">
      <div className="wsp-tabs">
        {panes.map((p) => (
          <div
            key={p.id}
            className={"wsp-tab" + (visible.has(p.id) ? " on" : "")}
            onClick={() => setActive(p.id)}
            title={`${p.title}\n工作区 ${p.ws}（持久：关掉不影响远端）`}
          >
            <span className={"dot" + (p.conn === true ? " on" : p.conn === false ? " off" : "")} />
            <span className="lbl">{tabLabel(p)}</span>
            <button
              className="x"
              title="关闭（远端的活继续跑）"
              onClick={(e) => { e.stopPropagation(); closePane(p.id); }}
            ><Icon name="x" /></button>
          </div>
        ))}
        {cur && (
          <button
            className="wsp-new"
            onClick={() => openPane(cur.server, { fresh: true })}
            title={`在 ${cur.server} 上再开一个终端`}
          ><Icon name="plus" /></button>
        )}
        {panes.length > 1 && (
          <button
            className={"wsp-split" + (split ? " on" : "")}
            onClick={() => setSplit((s) => !s)}
            title={split ? "取消分屏" : "分屏（并排两个）"}
          ><Icon name="grid" />{split ? "取消分屏" : "分屏"}</button>
        )}
      </div>

      <div className={"wsp-body" + (split ? " split" : "")}>
        {panes.length === 0 && (
          <div className="wsp-empty">{empty ?? "在左侧织物图里点一个节点，这里开出它的终端。"}</div>
        )}
        {panes.map((p) => (
          <div key={p.id} className={"tpane" + (visible.has(p.id) ? "" : " hidden")}>
            <TermView
              server={p.server}
              ws={p.ws}
              embedded
              onStatus={(c) => patch(p.id, { conn: c })}
              onTitle={(t) => patch(p.id, { title: t })}
            />
          </div>
        ))}
      </div>
    </div>
  );
});
