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
  compute: boolean; // 人节点:是否也是算力(共享了自身设备);机器节点恒 true
  gate: boolean;    // 是不是「门」(subnet router 或被当跳板的机)
  me: boolean;      // 是不是「我」(当前登录身份)
  x: number;
  y: number;
  vx: number;
  vy: number;
  card: { title: string; type: string; lines: string[] };
}
interface E { a: string; b: string }

// 一台算力节点在图里的目标 id:自身设备折进 owner 的人节点,服务器是独立机器节点。
function target(mc: TeamView["machines"][number]): string {
  return mc.is_self ? `p:${mc.owner}` : `m:${mc.name}`;
}

// IPv4 是否落在某 CIDR 内(判断哪些机在 subnet router 广播的网段后面)。
function ipInCidr(ip: string, cidr: string): boolean {
  const [net, bitsStr] = cidr.split("/");
  if (!/^\d+\.\d+\.\d+\.\d+$/.test(ip) || !net || !/^\d+\.\d+\.\d+\.\d+$/.test(net)) return false;
  const bits = Math.min(32, Math.max(0, parseInt(bitsStr ?? "32", 10) || 0));
  const toInt = (s: string) => s.split(".").reduce((a, o) => ((a << 8) + (parseInt(o, 10) || 0)) >>> 0, 0);
  const mask = bits === 0 ? 0 : (~0 << (32 - bits)) >>> 0;
  return (toInt(ip) & mask) === (toInt(net) & mask);
}

function build(view: TeamView, me?: string): { nodes: N[]; edges: E[] } {
  // 「我」= 身份(SSO login)或账号名对得上的成员节点。
  const meKey = (me || "").trim().toLowerCase();
  const isMe = (m: { name: string; identity: string }) =>
    !!meKey && (m.identity.toLowerCase() === meKey || m.name.toLowerCase() === meKey);
  const nodes: N[] = [];
  const edges: E[] = [];
  const selfDev = new Map(view.machines.filter((mc) => mc.is_self).map((mc) => [mc.owner, mc]));
  const servers = view.machines.filter((mc) => !mc.is_self);
  const N = view.members.length + servers.length;
  let k = 0;
  const place = () => {
    // 环形初始撒点 + 抖动,力导向再散成网。
    const ang = (k / Math.max(1, N)) * Math.PI * 2;
    k++;
    return { x: 0.5 + Math.cos(ang) * 0.28 + (Math.random() - 0.5) * 0.06, y: 0.5 + Math.sin(ang) * 0.28 + (Math.random() - 0.5) * 0.06 };
  };
  const reachersOf = (mc: TeamView["machines"][number]) =>
    view.members.filter((m) => m.name !== mc.owner && (mc.grants[m.role] ?? 0) > 0).map((m) => `${m.name}(档${mc.grants[m.role]})`);

  // 门(网关):subnet router(广播了子网)或被别的机当跳板的机。第一等节点。
  const jumpTargets = new Set(view.machines.map((m) => m.jump).filter(Boolean) as string[]);
  const isGate = (mc: TeamView["machines"][number]) => (mc.advertises?.length ?? 0) > 0 || jumpTargets.has(mc.name);
  // 一台机经哪道门可达:它的 jump 指向门,或它的 host 落在某门广播的网段内。
  const gateOf = (mc: TeamView["machines"][number]): string | null => {
    if (mc.jump && jumpTargets.has(mc.jump)) return mc.jump;
    for (const g of view.machines) {
      if (g.name === mc.name) continue;
      if ((g.advertises ?? []).some((c) => ipInCidr(mc.host, c))) return g.name;
    }
    return null;
  };

  // 人节点(含算力切面)
  for (const m of view.members) {
    const dev = selfDev.get(m.name);
    const ownServers = servers.filter((mc) => mc.owner === m.name).map((mc) => mc.name);
    // 我能进的:所有算力节点(服务器 + 别人的设备),按角色算档。设备用 owner 名指代。
    const canReach = view.machines
      .filter((mc) => mc.owner !== m.name && (mc.grants[m.role] ?? 0) > 0)
      .map((mc) => `${mc.is_self ? mc.owner : mc.name}(档${mc.grants[m.role]})`);
    const lines: string[] = [];
    if (dev) lines.push(`本机算力 ${dev.host} · ${reachersOf(dev).length ? "可进 " + reachersOf(dev).join("、") : "仅自己"}`);
    if (ownServers.length) lines.push(`贡献服务器 ${ownServers.join("、")}`);
    if (canReach.length) lines.push(`可访问 ${canReach.join("、")}`);
    if (!lines.length) lines.push(dev ? "只共享本机" : "仅消费(未共享算力)");
    const pos = place();
    const mine = isMe(m);
    nodes.push({
      id: `p:${m.name}`, name: m.name, kind: "person", compute: !!dev, gate: false, me: mine, ...pos, vx: 0, vy: 0,
      card: { title: mine ? `${m.name}（我）` : m.name, type: dev ? `人 · ${m.role} · 算力` : `人 · ${m.role}`, lines },
    });
  }
  // 服务器节点(独立算力,非本人)
  for (const mc of servers) {
    const reachers = reachersOf(mc);
    const gate = isGate(mc);
    const via = gateOf(mc);
    const lines: string[] = [];
    if (gate && (mc.advertises?.length ?? 0) > 0) {
      lines.push(`网关 · 广播 ${mc.advertises.join("、")}`);
      const behind = servers.filter((s) => s.name !== mc.name && (mc.advertises ?? []).some((c) => ipInCidr(s.host, c)));
      if (behind.length) lines.push(`${behind.length} 台经它可达: ${behind.map((s) => s.name).join("、")}`);
    } else if (gate) {
      lines.push("跳板 · 别的机经它进内网");
    }
    if (mc.owner) lines.push(`属于 ${mc.owner}`);
    if (via) lines.push(`经门 ${via} 可达`);
    lines.push(reachers.length ? `可进 ${reachers.join("、")}` : "无人可进");
    const pos = place();
    nodes.push({ id: `m:${mc.name}`, name: mc.name, kind: "machine", compute: true, gate, me: false, ...pos, vx: 0, vy: 0, card: { title: mc.name, type: gate ? `门 · ${mc.host}` : `机器 · ${mc.host}`, lines } });
    if (mc.owner) edges.push({ a: `p:${mc.owner}`, b: `m:${mc.name}` }); // 归属(自身设备无此边,已折进人)
  }
  // 授权边:每个算力节点 ← 能进它的人(自身设备的边指向 owner 人节点 = 人→人)。
  const nodeIds = new Set(nodes.map((n) => n.id));
  for (const mc of view.machines) {
    const tgt = target(mc);
    if (!nodeIds.has(tgt)) continue;
    for (const m of view.members) {
      if (m.name === mc.owner) continue;
      if ((mc.grants[m.role] ?? 0) > 0 && nodeIds.has(`p:${m.name}`)) edges.push({ a: `p:${m.name}`, b: tgt });
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
      ink: col("--text-strong"),
      faint: col("--text-faint"),
      surface: col("--surface-raised"),
      bd: col("--border-default"),
    });

    const idx = new Map(nodes.map((n, i) => [n.id, i]));
    const nbr = new Map<string, Set<string>>(nodes.map((n) => [n.id, new Set<string>()]));
    for (const e of edges) { nbr.get(e.a)?.add(e.b); nbr.get(e.b)?.add(e.a); }

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
        // 「人+算力」环:人节点若共享了自身设备,外描一圈算力色 = 他本人就是一台算力。
        if (n.kind === "person" && n.compute) {
          ctx.beginPath(); ctx.arc(x, y, r + 2.4, 0, Math.PI * 2);
          ctx.strokeStyle = hexA(p.machine, dim ? 0.3 : 0.85); ctx.lineWidth = 2; ctx.stroke();
        }
        // 「门」标记:subnet router / 跳板,外描一圈主色菱形(与圆环区分)= 一道进内网的门。
        if (n.gate) {
          const gr = r + 5;
          ctx.beginPath();
          ctx.moveTo(x, y - gr); ctx.lineTo(x + gr, y); ctx.lineTo(x, y + gr); ctx.lineTo(x - gr, y); ctx.closePath();
          ctx.strokeStyle = hexA(p.dotHi, dim ? 0.3 : 0.9); ctx.lineWidth = 1.6; ctx.stroke();
        }
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
        <span><i className="d compute" />人+算力</span>
        <span><i className="d machine" />机器</span>
        <span><i className="d gate" />门(网关/跳板)</span>
        <span><i className="d me" />我</span>
        <span className="hint">悬停查看身份与授权</span>
      </div>
    </div>
  );
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
