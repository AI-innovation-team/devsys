import { useCallback, useImperativeHandle, useRef, useState, forwardRef } from "react";

import { TermView } from "../components/TermView";
import { Icon } from "../icons";

// 工作区 = 驾驶舱的「工作面」：多个活终端 pane，标签切换 + 可分屏。
// 每个 pane 一条独立 SSH 会话（后端 Sessions 按 id 支持 N 并发）。
// 关键：非活动 pane **保持挂载**、只用 CSS 隐藏 —— 卸载会关掉会话、丢掉正在跑的活。
export interface Pane {
  id: string;
  server: string;
  ws: string;
  title: string;
  conn: boolean | null;
}

export interface WorkspaceHandle {
  openPane: (server: string, ws?: string) => void;
}

let seq = 1;

export const Workspace = forwardRef<WorkspaceHandle, { empty?: React.ReactNode }>(function Workspace(
  { empty },
  ref,
) {
  const [panes, setPanes] = useState<Pane[]>([]);
  const [active, setActive] = useState<string>("");
  const [split, setSplit] = useState(false);
  const activeRef = useRef(active);
  activeRef.current = active;

  const openPane = useCallback((server: string, ws = "") => {
    setPanes((cur) => {
      // 同一台机 + 同一工作区已开着 → 只切过去，不重复开会话。
      const hit = cur.find((p) => p.server === server && p.ws === ws);
      if (hit) { setActive(hit.id); return cur; }
      const id = `pane-${seq++}`;
      setActive(id);
      return [...cur, { id, server, ws, title: ws ? `${server} · ${ws}` : server, conn: null }];
    });
  }, []);

  useImperativeHandle(ref, () => ({ openPane }), [openPane]);

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

  return (
    <div className="wsp">
      <div className="wsp-tabs">
        {panes.map((p) => (
          <div
            key={p.id}
            className={"wsp-tab" + (visible.has(p.id) ? " on" : "")}
            onClick={() => setActive(p.id)}
            title={p.title}
          >
            <span className={"dot" + (p.conn === true ? " on" : p.conn === false ? " off" : "")} />
            <span className="lbl">{p.server}</span>
            <button
              className="x"
              title="关闭"
              onClick={(e) => { e.stopPropagation(); closePane(p.id); }}
            ><Icon name="x" /></button>
          </div>
        ))}
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
