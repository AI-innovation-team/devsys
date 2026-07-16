import { useEffect, useMemo, useRef } from "react";

import { type TeamView } from "../data";

// 团队拓扑图 = 一张织物网。北极星「没有本地/远程之分,只有节点」的画面。
//   · 所有节点(人 / 机器)都是平等的小圆点 —— 类型不靠形状/颜色区分,悬停才展开。
//   · 边不分类型,统一素线 —— 关系细节(贡献/授权/档位)在悬停卡里看。
//   · org 不是圆心节点,而是一圈柔和「包络」—— 有组织但无特权中心 = 真 P2P。
//     多 org 就是并排多个包络(联邦);包络外的自由节点 = 公地。此处单 org 单包络。
interface N {
  id: string;
  name: string;
  kind: "person" | "machine";
  group: string; // 所属 org(包络分组);未来联邦=多组
  me: boolean;   // 是不是「我」(当前登录身份)
  x: number;
  y: number;
  vx: number;
  vy: number;
  card: { title: string; type: string; lines: string[] };
}
interface E { a: string; b: string }

function build(view: TeamView, me?: string): { nodes: N[]; edges: E[] } {
  // 「我」= 身份(SSO login)或账号名对得上的成员节点。
  const meKey = (me || "").trim().toLowerCase();
  const isMe = (m: { name: string; identity: string }) =>
    !!meKey && (m.identity.toLowerCase() === meKey || m.name.toLowerCase() === meKey);
  const nodes: N[] = [];
  const edges: E[] = [];
  const has = new Set<string>();
  const N = view.members.length + view.machines.length;
  let k = 0;
  const place = () => {
    // 环形初始撒点 + 抖动,力导向再散成网。
    const ang = (k / Math.max(1, N)) * Math.PI * 2;
    k++;
    return { x: 0.5 + Math.cos(ang) * 0.28 + (Math.random() - 0.5) * 0.06, y: 0.5 + Math.sin(ang) * 0.28 + (Math.random() - 0.5) * 0.06 };
  };

  for (const m of view.members) {
    const owns = view.machines.filter((mc) => mc.owner === m.name).map((mc) => mc.name);
    const access = view.machines
      .filter((mc) => mc.owner !== m.name && (mc.grants[m.role] ?? 0) > 0)
      .map((mc) => `${mc.name}(档${mc.grants[m.role]})`);
    const lines: string[] = [];
    if (owns.length) lines.push(`贡献 ${owns.join("、")}`);
    if (access.length) lines.push(`可访问 ${access.join("、")}`);
    if (!lines.length) lines.push("无授权(仅本人节点)");
    const pos = place();
    const mine = isMe(m);
    nodes.push({ id: `p:${m.name}`, name: m.name, kind: "person", group: view.team, me: mine, ...pos, vx: 0, vy: 0, card: { title: mine ? `${m.name}（我）` : m.name, type: `人 · ${m.role}`, lines } });
    has.add(`p:${m.name}`);
  }
  for (const mc of view.machines) {
    const reachers = view.members
      .filter((m) => m.name !== mc.owner && (mc.grants[m.role] ?? 0) > 0)
      .map((m) => `${m.name}(档${mc.grants[m.role]})`);
    const lines: string[] = [];
    if (mc.owner) lines.push(`属于 ${mc.owner}`);
    lines.push(reachers.length ? `可进 ${reachers.join("、")}` : "无人可进");
    const pos = place();
    nodes.push({ id: `m:${mc.name}`, name: mc.name, kind: "machine", group: view.team, me: false, ...pos, vx: 0, vy: 0, card: { title: mc.name, type: `机器 · ${mc.host}`, lines } });
    has.add(`m:${mc.name}`);
    if (mc.owner && has.has(`p:${mc.owner}`)) edges.push({ a: `p:${mc.owner}`, b: `m:${mc.name}` });
    for (const m of view.members) {
      if (m.name === mc.owner) continue;
      if ((mc.grants[m.role] ?? 0) > 0) edges.push({ a: `p:${m.name}`, b: `m:${mc.name}` });
    }
  }
  return { nodes, edges };
}

export function TeamGraph({ view, me }: { view: TeamView; me?: string }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const hover = useRef<string | null>(null);

  const count = useMemo(() => view.members.length + view.machines.length, [view]);
  const height = Math.min(560, Math.max(300, 240 + count * 20));

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const { nodes, edges } = build(view, me);
    if (!nodes.length) return;
    const meId = nodes.find((n) => n.me)?.id ?? null;

    const css = getComputedStyle(document.documentElement);
    const col = (v: string) => css.getPropertyValue(v).trim() || "#888";
    const pal = () => ({
      person: col("--success"),  // sage 绿 = 人
      machine: col("--info"),     // slate 蓝 = 机器
      dotHi: col("--accent"),     // 焦点/我/高亮边 = 主色
      edge: col("--border-strong"),
      hull: col("--accent"),
      ink: col("--text-strong"),
      faint: col("--text-faint"),
      surface: col("--surface-raised"),
      bd: col("--border-default"),
    });

    const idx = new Map(nodes.map((n, i) => [n.id, i]));
    const nbr = new Map<string, Set<string>>(nodes.map((n) => [n.id, new Set<string>()]));
    for (const e of edges) { nbr.get(e.a)?.add(e.b); nbr.get(e.b)?.add(e.a); }
    // 按 org 分组(将来联邦=多组,各画一个包络)。
    const groups = new Map<string, number[]>();
    nodes.forEach((n, i) => { (groups.get(n.group) ?? groups.set(n.group, []).get(n.group)!).push(i); });

    let raf = 0, settle = 0;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const resize = () => { canvas.width = canvas.clientWidth * dpr; canvas.height = canvas.clientHeight * dpr; };
    resize();

    const step = () => {
      const W = canvas.clientWidth, H = canvas.clientHeight;
      // 力导向织物:互斥 + 边弹簧 + 向心。无分列锚 —— 长成一张网,不是层级。
      for (let i = 0; i < nodes.length; i++) {
        const a = nodes[i];
        for (let j = i + 1; j < nodes.length; j++) {
          const b = nodes[j];
          let dx = a.x - b.x, dy = a.y - b.y;
          const d2 = dx * dx + dy * dy + 0.001;
          const f = 0.0012 / d2;
          dx *= f; dy *= f;
          a.vx += dx; a.vy += dy; b.vx -= dx; b.vy -= dy;
        }
      }
      for (const e of edges) {
        const a = nodes[idx.get(e.a)!], b = nodes[idx.get(e.b)!];
        if (!a || !b) continue;
        const dx = b.x - a.x, dy = b.y - a.y;
        const f = ((Math.hypot(dx, dy) || 0.001) - 0.17) * 0.02;
        a.vx += dx * f; a.vy += dy * f; b.vx -= dx * f; b.vy -= dy * f;
      }
      for (const n of nodes) {
        n.vx += (0.5 - n.x) * 0.006; n.vy += (0.5 - n.y) * 0.006; // 向心
        n.vx *= 0.85; n.vy *= 0.85;
        n.x += n.vx; n.y += n.vy;
        n.x = Math.max(0.12, Math.min(0.88, n.x));
        n.y = Math.max(0.14, Math.min(0.86, n.y));
      }

      // ── 渲染 ──
      const p = pal();
      const ctx = canvas.getContext("2d")!;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, W, H);
      ctx.lineCap = "round";
      const X = (v: number) => v * W, Y = (v: number) => v * H;
      const hv = hover.current;
      // 焦点:悬停优先;没悬停时「我」是软焦点(默认微微亮出我能进的机器)。
      const focus = hv ?? meId;
      const strong = !!hv; // 悬停=强聚焦(其余狠狠淡出);「我」软焦点只轻轻提亮
      const near = focus ? nbr.get(focus) : null;

      // org 包络(压在最底;不标名,一片无名领地即可)
      for (const ids of groups.values()) {
        const pts = ids.map((i) => ({ x: X(nodes[i].x), y: Y(nodes[i].y) }));
        drawHull(ctx, pts, 30, hexA(p.hull, 0.07), hexA(p.hull, 0.28));
      }

      // 边(统一素线,不分类型;轻微弧线,密集时不打架)
      for (const e of edges) {
        const a = nodes[idx.get(e.a)!], b = nodes[idx.get(e.b)!];
        if (!a || !b) continue;
        const on = focus && (a.id === focus || b.id === focus);
        const ax = X(a.x), ay = Y(a.y), bx = X(b.x), by = Y(b.y);
        // 中垂法向偏移出控制点 → 二次贝塞尔轻弧。
        const dx = bx - ax, dy = by - ay, len = Math.hypot(dx, dy) || 1;
        const cx = (ax + bx) / 2 + (-dy / len) * len * 0.12;
        const cy = (ay + by) / 2 + (dx / len) * len * 0.12;
        ctx.beginPath(); ctx.moveTo(ax, ay); ctx.quadraticCurveTo(cx, cy, bx, by);
        if (on) { ctx.strokeStyle = hexA(p.dotHi, strong ? 0.75 : 0.42); ctx.lineWidth = strong ? 1.9 : 1.5; }
        else if (focus) { ctx.strokeStyle = hexA(p.edge, strong ? 0.1 : 0.42); ctx.lineWidth = 1.1; }
        else { ctx.strokeStyle = hexA(p.edge, 0.5); ctx.lineWidth = 1.2; }
        ctx.stroke();
      }

      // 节点(统一小圆点;类型不靠外观区分。「我」多一圈主色环)
      for (const n of nodes) {
        const x = X(n.x), y = Y(n.y);
        const isFocus = focus === n.id, isNb = !!near?.has(n.id);
        const dim = strong && !isFocus && !isNb; // 强聚焦时无关节点狠狠淡出
        const r = isFocus ? 8 : 5.5;
        // 「我」环:始终在点外描一圈主色,一眼找到自己。
        if (n.me) {
          ctx.beginPath(); ctx.arc(x, y, r + 4, 0, Math.PI * 2);
          ctx.strokeStyle = hexA(p.dotHi, dim ? 0.3 : 0.9); ctx.lineWidth = 1.6; ctx.stroke();
        }
        const base = n.kind === "person" ? p.person : p.machine; // 类型色始终在
        if (isFocus && strong) { ctx.shadowColor = hexA(base, 0.65); ctx.shadowBlur = 12; }
        ctx.beginPath(); ctx.arc(x, y, r, 0, Math.PI * 2);
        ctx.fillStyle = dim ? hexA(base, 0.28) : base;
        ctx.fill(); ctx.shadowBlur = 0;
        // 名字标签:强聚焦时只留焦点(卡片)+邻居;其余隐去。软焦点/静态全显。
        const showLabel = !(isFocus && strong) && !dim && !(strong && !isNb && !isFocus);
        if (showLabel) {
          ctx.fillStyle = n.me ? hexA(p.dotHi, 0.95) : p.faint;
          ctx.font = `${n.me ? "600" : "500"} 10px system-ui, sans-serif`;
          ctx.textAlign = "center"; ctx.textBaseline = "top";
          ctx.fillText(clip(n.name, 10), x, y + r + 3);
        }
      }

      // 悬停详情卡(最上层)
      if (hv) {
        const n = nodes[idx.get(hv)!];
        if (n) drawCard(ctx, X(n.x), Y(n.y), W, H, n.card, p);
      }

      if (settle++ < 300) raf = requestAnimationFrame(step); else raf = 0;
    };
    step();

    const onMove = (ev: MouseEvent) => {
      const rect = canvas.getBoundingClientRect();
      const mx = (ev.clientX - rect.left) / rect.width, my = (ev.clientY - rect.top) / rect.height;
      let best: string | null = null, bd = 0.045;
      for (const n of nodes) { const d = Math.hypot(n.x - mx, n.y - my); if (d < bd) { bd = d; best = n.id; } }
      if (best !== hover.current) { hover.current = best; settle = Math.min(settle, 280); if (!raf) step(); }
      canvas.style.cursor = best ? "pointer" : "default";
    };
    canvas.addEventListener("mousemove", onMove);
    const onLeave = () => { if (hover.current) { hover.current = null; settle = Math.min(settle, 280); if (!raf) step(); } };
    canvas.addEventListener("mouseleave", onLeave);
    const onResize = () => { resize(); settle = Math.min(settle, 260); if (!raf) step(); };
    window.addEventListener("resize", onResize);

    return () => { cancelAnimationFrame(raf); canvas.removeEventListener("mousemove", onMove); canvas.removeEventListener("mouseleave", onLeave); window.removeEventListener("resize", onResize); };
  }, [view, me]);

  return (
    <div className="tgraph">
      <canvas ref={ref} style={{ height }} />
      <div className="tgraph-legend">
        <span><i className="d person" />人</span>
        <span><i className="d machine" />机器</span>
        <span><i className="d me" />我</span>
        <span className="hint">一圈包络 = 团队(无中心) · 悬停查看身份与授权</span>
      </div>
    </div>
  );
}

// 凸包 → 外扩 → 平滑闭合曲线,画成一片柔和领地。
function drawHull(ctx: CanvasRenderingContext2D, pts: { x: number; y: number }[], pad: number, fill: string, stroke: string) {
  if (pts.length < 3) {
    // 太少节点:退化成一个包住它们的圆角框。
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const q of pts) { x0 = Math.min(x0, q.x); y0 = Math.min(y0, q.y); x1 = Math.max(x1, q.x); y1 = Math.max(y1, q.y); }
    ctx.beginPath();
    const r = pad;
    rr(ctx, x0 - pad, y0 - pad, x1 - x0 + pad * 2, y1 - y0 + pad * 2, r);
    ctx.fillStyle = fill; ctx.fill(); ctx.strokeStyle = stroke; ctx.lineWidth = 1.5; ctx.stroke();
    return;
  }
  const hull = convexHull(pts);
  let cx = 0, cy = 0; for (const q of hull) { cx += q.x; cy += q.y; } cx /= hull.length; cy /= hull.length;
  const ex = hull.map((q) => { const dx = q.x - cx, dy = q.y - cy, d = Math.hypot(dx, dy) || 1; return { x: q.x + (dx / d) * pad, y: q.y + (dy / d) * pad }; });
  ctx.beginPath();
  const n = ex.length;
  const mid = (i: number, j: number) => ({ x: (ex[i].x + ex[j].x) / 2, y: (ex[i].y + ex[j].y) / 2 });
  let m0 = mid(n - 1, 0);
  ctx.moveTo(m0.x, m0.y);
  for (let i = 0; i < n; i++) { const m1 = mid(i, (i + 1) % n); ctx.quadraticCurveTo(ex[i].x, ex[i].y, m1.x, m1.y); }
  ctx.closePath();
  ctx.fillStyle = fill; ctx.fill();
  ctx.strokeStyle = stroke; ctx.lineWidth = 1.5; ctx.setLineDash([6, 5]); ctx.stroke(); ctx.setLineDash([]);
}

function convexHull(pts: { x: number; y: number }[]): { x: number; y: number }[] {
  const p = [...pts].sort((a, b) => a.x - b.x || a.y - b.y);
  const cross = (o: any, a: any, b: any) => (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
  const lo: any[] = [];
  for (const q of p) { while (lo.length >= 2 && cross(lo[lo.length - 2], lo[lo.length - 1], q) <= 0) lo.pop(); lo.push(q); }
  const up: any[] = [];
  for (let i = p.length - 1; i >= 0; i--) { const q = p[i]; while (up.length >= 2 && cross(up[up.length - 2], up[up.length - 1], q) <= 0) up.pop(); up.push(q); }
  lo.pop(); up.pop();
  return lo.concat(up);
}

function drawCard(ctx: CanvasRenderingContext2D, nx: number, ny: number, W: number, H: number, card: { title: string; type: string; lines: string[] }, p: any) {
  ctx.font = "600 12px system-ui, sans-serif";
  const widths = [ctx.measureText(card.title).width];
  ctx.font = "500 11px system-ui, sans-serif";
  widths.push(ctx.measureText(card.type).width, ...card.lines.map((l) => ctx.measureText(l).width));
  const w = Math.min(240, Math.max(...widths) + 24);
  const h = 20 + 16 + card.lines.length * 15 + 10;
  let x = nx + 14, y = ny - h / 2;
  if (x + w > W - 6) x = nx - 14 - w;
  x = Math.max(6, Math.min(W - w - 6, x));
  y = Math.max(6, Math.min(H - h - 6, y));
  ctx.shadowColor = "rgba(0,0,0,0.18)"; ctx.shadowBlur = 14; ctx.shadowOffsetY = 3;
  rr(ctx, x, y, w, h, 10); ctx.fillStyle = p.surface; ctx.fill();
  ctx.shadowBlur = 0; ctx.shadowOffsetY = 0;
  ctx.strokeStyle = p.bd; ctx.lineWidth = 1; rr(ctx, x, y, w, h, 10); ctx.stroke();
  ctx.textAlign = "left"; ctx.textBaseline = "top";
  ctx.fillStyle = p.ink; ctx.font = "600 12px system-ui, sans-serif";
  ctx.fillText(clip(card.title, 22), x + 12, y + 11);
  ctx.fillStyle = p.faint; ctx.font = "500 11px system-ui, sans-serif";
  ctx.fillText(clip(card.type, 26), x + 12, y + 27);
  ctx.fillStyle = col2(p.ink, 0.82);
  card.lines.forEach((l, i) => ctx.fillText(clip(l, 30), x + 12, y + 45 + i * 15));
}

function rr(c: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  c.beginPath();
  c.moveTo(x + r, y); c.arcTo(x + w, y, x + w, y + h, r); c.arcTo(x + w, y + h, x, y + h, r);
  c.arcTo(x, y + h, x, y, r); c.arcTo(x, y, x + w, y, r); c.closePath();
}
function clip(s: string, n: number) { return s.length > n ? s.slice(0, n - 1) + "…" : s; }
function hexA(hex: string, a: number): string {
  const h = hex.replace("#", "");
  if (h.length < 6) return hex;
  const r = parseInt(h.slice(0, 2), 16), g = parseInt(h.slice(2, 4), 16), b = parseInt(h.slice(4, 6), 16);
  return `rgba(${r},${g},${b},${a})`;
}
function col2(hex: string, a: number) { return hexA(hex, a); }
