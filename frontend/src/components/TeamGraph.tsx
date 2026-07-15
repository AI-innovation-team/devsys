import { useEffect, useRef } from "react";

import { type TeamView } from "../data";

// 团队拓扑图。分文件 + RBAC 之后，数据天然是一张图 —— 这里把它画出来。
// 节点：人 / 机器 / 角色；边：贡献（人→机）、授权（机→角色，粗细=档位）、属于（人→角色）。
// 这正是北极星「只有节点，只有织物」的画面。力导向布局，Canvas 渲染。
type NodeT = "person" | "machine" | "role";
interface N {
  id: string;
  label: string;
  t: NodeT;
  x: number;
  y: number;
  vx: number;
  vy: number;
  sub?: string;
}
interface E {
  a: string;
  b: string;
  kind: "contrib" | "grant" | "belong";
  tier?: number;
}

function build(view: TeamView): { nodes: N[]; edges: E[] } {
  const nodes: N[] = [];
  const edges: E[] = [];
  const seen = new Set<string>();
  const add = (id: string, label: string, t: NodeT, sub?: string) => {
    if (seen.has(id)) return;
    seen.add(id);
    // 初始位置按类型分三簇（人左、机中、角色右），力导向再散开。
    const bx = t === "person" ? 0.22 : t === "machine" ? 0.5 : 0.78;
    nodes.push({ id, label, t, sub, x: bx + (Math.random() - 0.5) * 0.12, y: 0.2 + Math.random() * 0.6, vx: 0, vy: 0 });
  };

  for (const r of Object.keys(view.roles)) add(`role:${r}`, r, "role", `档 ${view.roles[r].tier}`);
  for (const m of view.members) {
    add(`person:${m.name}`, m.name, "person", m.role);
    edges.push({ a: `person:${m.name}`, b: `role:${m.role}`, kind: "belong" });
  }
  for (const mc of view.machines) {
    add(`machine:${mc.name}`, mc.name, "machine", mc.host);
    if (mc.owner) edges.push({ a: `person:${mc.owner}`, b: `machine:${mc.name}`, kind: "contrib" });
    for (const [role, tier] of Object.entries(mc.grants)) {
      if (seen.has(`role:${role}`)) edges.push({ a: `machine:${mc.name}`, b: `role:${role}`, kind: "grant", tier });
    }
  }
  return { nodes, edges };
}

export function TeamGraph({ view }: { view: TeamView }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const hover = useRef<string | null>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const { nodes, edges } = build(view);
    if (!nodes.length) return;

    const css = getComputedStyle(document.documentElement);
    const col = (v: string) => css.getPropertyValue(v).trim() || "#888";
    const palette = () => ({
      person: col("--success"),
      machine: col("--text-muted"),
      role: col("--accent"),
      line: col("--border-default"),
      accent: col("--accent"),
      ink: col("--text-strong"),
      faint: col("--text-faint"),
      surface: col("--surface"),
    });

    const idx = new Map(nodes.map((n, i) => [n.id, i]));
    let raf = 0;
    let settle = 0;

    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const resize = () => {
      const w = canvas.clientWidth, h = canvas.clientHeight;
      canvas.width = w * dpr; canvas.height = h * dpr;
    };
    resize();

    const step = () => {
      const W = canvas.clientWidth, H = canvas.clientHeight;
      // 力导向：节点互斥 + 边弹簧 + 按类型分列的水平锚 + 居中。
      for (let i = 0; i < nodes.length; i++) {
        const a = nodes[i];
        for (let j = i + 1; j < nodes.length; j++) {
          const b = nodes[j];
          let dx = a.x - b.x, dy = a.y - b.y;
          let d2 = dx * dx + dy * dy + 0.001;
          const f = 0.0009 / d2;
          dx *= f; dy *= f;
          a.vx += dx; a.vy += dy; b.vx -= dx; b.vy -= dy;
        }
      }
      for (const e of edges) {
        const a = nodes[idx.get(e.a)!], b = nodes[idx.get(e.b)!];
        if (!a || !b) continue;
        const dx = b.x - a.x, dy = b.y - a.y;
        const target = 0.16;
        const f = ((Math.hypot(dx, dy) || 0.001) - target) * 0.02;
        const ux = dx * f, uy = dy * f;
        a.vx += ux; a.vy += uy; b.vx -= ux; b.vy -= uy;
      }
      for (const n of nodes) {
        const anchor = n.t === "person" ? 0.2 : n.t === "machine" ? 0.5 : 0.8;
        n.vx += (anchor - n.x) * 0.02; // 水平分列
        n.vy += (0.5 - n.y) * 0.004;   // 轻微垂直居中
        n.vx *= 0.86; n.vy *= 0.86;
        n.x += n.vx; n.y += n.vy;
        n.x = Math.max(0.06, Math.min(0.94, n.x));
        n.y = Math.max(0.08, Math.min(0.92, n.y));
      }

      // ── 渲染 ──
      const p = palette();
      const ctx = canvas.getContext("2d")!;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, W, H);
      const X = (v: number) => v * W, Y = (v: number) => v * H;
      const hv = hover.current;

      // 边
      for (const e of edges) {
        const a = nodes[idx.get(e.a)!], b = nodes[idx.get(e.b)!];
        if (!a || !b) continue;
        const on = hv && (a.id === hv || b.id === hv);
        ctx.beginPath();
        ctx.moveTo(X(a.x), Y(a.y));
        ctx.lineTo(X(b.x), Y(b.y));
        if (e.kind === "grant") {
          ctx.strokeStyle = on ? p.accent : hexA(p.accent, 0.4);
          ctx.lineWidth = e.tier === 0 ? 0.6 : e.tier === 2 ? 2.4 : 1.4;
          ctx.setLineDash([4, 3]);
        } else if (e.kind === "belong") {
          ctx.strokeStyle = on ? p.faint : hexA(p.faint, 0.5);
          ctx.lineWidth = 1; ctx.setLineDash([2, 3]);
        } else {
          ctx.strokeStyle = on ? p.ink : p.line;
          ctx.lineWidth = 1.4; ctx.setLineDash([]);
        }
        ctx.stroke();
      }
      ctx.setLineDash([]);

      // 节点
      for (const n of nodes) {
        const x = X(n.x), y = Y(n.y);
        const on = hv === n.id;
        ctx.fillStyle = n.t === "person" ? p.person : n.t === "machine" ? p.surface : hexA(p.role, 0.16);
        ctx.strokeStyle = n.t === "person" ? p.person : n.t === "machine" ? p.machine : p.role;
        ctx.lineWidth = on ? 2.4 : 1.5;
        if (n.t === "machine") {
          roundRect(ctx, x - 34, y - 13, 68, 26, 7); ctx.fill(); ctx.stroke();
        } else if (n.t === "role") {
          roundRect(ctx, x - 26, y - 12, 52, 24, 12); ctx.fill(); ctx.stroke();
        } else {
          ctx.beginPath(); ctx.arc(x, y, on ? 15 : 13, 0, Math.PI * 2); ctx.fill(); ctx.stroke();
        }
        ctx.fillStyle = n.t === "person" ? p.surface : p.ink;
        ctx.font = `600 ${n.t === "person" ? 10 : 11}px system-ui, sans-serif`;
        ctx.textAlign = "center"; ctx.textBaseline = "middle";
        ctx.fillText(clip(n.label, 9), x, y);
        if (on && n.sub) {
          ctx.fillStyle = p.faint;
          ctx.font = "500 10px system-ui, sans-serif";
          ctx.fillText(n.sub, x, y + (n.t === "person" ? 26 : 22));
        }
      }

      if (settle++ < 320) raf = requestAnimationFrame(step);
    };
    step();

    const onMove = (ev: MouseEvent) => {
      const rect = canvas.getBoundingClientRect();
      const mx = (ev.clientX - rect.left) / rect.width, my = (ev.clientY - rect.top) / rect.height;
      let best: string | null = null, bd = 0.05;
      for (const n of nodes) {
        const d = Math.hypot(n.x - mx, n.y - my);
        if (d < bd) { bd = d; best = n.id; }
      }
      if (best !== hover.current) { hover.current = best; settle = Math.min(settle, 300); if (!raf) step(); }
      canvas.style.cursor = best ? "pointer" : "default";
    };
    canvas.addEventListener("mousemove", onMove);
    const onResize = () => { resize(); settle = 0; if (!raf) step(); };
    window.addEventListener("resize", onResize);

    return () => { cancelAnimationFrame(raf); canvas.removeEventListener("mousemove", onMove); window.removeEventListener("resize", onResize); };
  }, [view]);

  return (
    <div className="tgraph">
      <canvas ref={ref} />
      <div className="tgraph-legend">
        <span><i className="d person" />人</span>
        <span><i className="d machine" />机器</span>
        <span><i className="d role" />角色</span>
        <span className="hint">— 贡献　--- 授权(粗细=档位)　··· 属于</span>
      </div>
    </div>
  );
}

function roundRect(c: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  c.beginPath();
  c.moveTo(x + r, y);
  c.arcTo(x + w, y, x + w, y + h, r);
  c.arcTo(x + w, y + h, x, y + h, r);
  c.arcTo(x, y + h, x, y, r);
  c.arcTo(x, y, x + w, y, r);
  c.closePath();
}
function clip(s: string, n: number) { return s.length > n ? s.slice(0, n - 1) + "…" : s; }
function hexA(hex: string, a: number): string {
  const h = hex.replace("#", "");
  if (h.length < 6) return hex;
  const r = parseInt(h.slice(0, 2), 16), g = parseInt(h.slice(2, 4), 16), b = parseInt(h.slice(4, 6), 16);
  return `rgba(${r},${g},${b},${a})`;
}
