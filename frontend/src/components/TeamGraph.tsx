import { useEffect, useMemo, useRef } from "react";

import { LOCAL_NODE, type Fabric } from "../data";

// 显示名:保留名 ~local 画成「本机」(连接键仍是 ~local,别改 node.name)。
const disp = (name: string) => (name === LOCAL_NODE ? "本机" : name);

// 织物图 = 驾驶舱的地图。北极星「没有本地/远程之分,只有节点」的画面,但**以我为中心**:
//   · 「我」固定在圆心 —— 这张图回答的问题是「**我**在哪、**我**够得着谁」,不是抽象网。
//   · 其余节点按与我的关系分环:我的机器(近)→ 门 → 队友 → 队友的机器(远)。
//     力导向仍在(斥力+边弹簧),环只是温和的径向偏好 —— 长成有方向感的网,不是同心圆刻度盘。
//   · 节点大小随画布/密度自适应;类型仍不靠形状区分(悬停出卡),但**状态**直接画在点上:
//     已连接=常亮光环 · 可达=呼吸脉冲 · 不可达=灰。图一眼回答「现在谁能用」。
//   · org 不是圆心节点;多 org 联邦时是并排包络(v2)。
interface N {
  id: string;
  name: string;
  kind: "person" | "machine";
  compute: boolean; // 人节点:是否也是算力(共享了自身设备);机器节点恒 true
  gate: boolean;    // 是不是「门」(subnet router 或被当跳板的机)
  me: boolean;      // 是不是「我」(当前登录身份)
  ring: number;     // 与「我」的关系环(归一化目标半径;me=0)
  x: number;
  y: number;
  vx: number;
  vy: number;
  card: { title: string; type: string; lines: string[] };
}
interface E { a: string; b: string }

// 一台算力节点在图里的目标 id:自身设备折进 owner 的人节点,服务器是独立机器节点。
function target(mc: Fabric["machines"][number]): string {
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

// 关系环半径(归一化)。me=0 固定圆心。
const RING = { myMachine: 0.17, gate: 0.25, member: 0.31, farMachine: 0.36 };

function build(view: Fabric, me?: string): { nodes: N[]; edges: E[] } {
  // 「我」= 身份(SSO login)或账号名对得上的成员节点。
  const meKey = (me || "").trim().toLowerCase();
  const isMe = (m: { name: string; identity: string }) =>
    !!meKey && (m.identity.toLowerCase() === meKey || m.name.toLowerCase() === meKey);
  const nodes: N[] = [];
  const edges: E[] = [];
  const selfDev = new Map(view.machines.filter((mc) => mc.is_self).map((mc) => [mc.owner, mc]));
  const servers = view.machines.filter((mc) => !mc.is_self);
  const total = view.members.length + servers.length;
  let k = 0;
  const place = (ring: number) => {
    // 按目标环撒初始点 + 抖动,力导向再松弛成网 —— 少抖几百帧就稳。
    const ang = (k / Math.max(1, total)) * Math.PI * 2 + (Math.random() - 0.5) * 0.5;
    k++;
    const r = Math.max(0.05, ring);
    return { x: 0.5 + Math.cos(ang) * r + (Math.random() - 0.5) * 0.04, y: 0.5 + Math.sin(ang) * r + (Math.random() - 0.5) * 0.04 };
  };
  const reachersOf = (mc: Fabric["machines"][number]) =>
    view.members.filter((m) => m.name !== mc.owner && (mc.grants[m.role] ?? 0) > 0).map((m) => `${m.name}(档${mc.grants[m.role]})`);

  // 门(网关):subnet router(广播了子网)或被别的机当跳板的机。第一等节点。
  const jumpTargets = new Set(view.machines.map((m) => m.jump).filter(Boolean) as string[]);
  const isGate = (mc: Fabric["machines"][number]) => (mc.advertises?.length ?? 0) > 0 || jumpTargets.has(mc.name);
  // 一台机经哪道门可达:它的 jump 指向门,或它的 host 落在某门广播的网段内。
  const gateOf = (mc: Fabric["machines"][number]): string | null => {
    if (mc.jump && jumpTargets.has(mc.jump)) return mc.jump;
    for (const g of view.machines) {
      if (g.name === mc.name) continue;
      if ((g.advertises ?? []).some((c) => ipInCidr(mc.host, c))) return g.name;
    }
    return null;
  };

  const meMember = view.members.find(isMe);
  const meName = meMember?.name ?? "";

  // 人节点(含算力切面)
  for (const m of view.members) {
    const dev = selfDev.get(m.name);
    const ownServers = servers.filter((mc) => mc.owner === m.name).map((mc) => mc.name);
    const canReach = view.machines
      .filter((mc) => mc.owner !== m.name && (mc.grants[m.role] ?? 0) > 0)
      .map((mc) => `${mc.is_self ? mc.owner : mc.name}(档${mc.grants[m.role]})`);
    const lines: string[] = [];
    if (dev) lines.push(`本机算力 ${dev.host} · ${reachersOf(dev).length ? "可进 " + reachersOf(dev).join("、") : "仅自己"}`);
    if (ownServers.length) lines.push(`贡献服务器 ${ownServers.join("、")}`);
    if (canReach.length) lines.push(`可访问 ${canReach.join("、")}`);
    if (!lines.length) lines.push(dev ? "只共享本机" : "仅消费(未共享算力)");
    const mine = isMe(m);
    const ring = mine ? 0 : RING.member;
    nodes.push({
      id: `p:${m.name}`, name: m.name, kind: "person", compute: !!dev, gate: false, me: mine, ring,
      ...(mine ? { x: 0.5, y: 0.5 } : place(ring)), vx: 0, vy: 0,
      card: { title: mine ? `${m.name}（我）` : m.name, type: dev ? `人 · ${m.role} · 算力` : `人 · ${m.role}`, lines },
    });
  }
  // 没匹配到成员的「我」(纯本地/未入团队):也造一个中心节点 —— 图永远以我为中心。
  const meId = meName ? `p:${meName}` : meKey ? "p:~me" : null;
  if (meId === "p:~me") {
    nodes.push({
      id: "p:~me", name: me!.trim(), kind: "person", compute: false, gate: false, me: true, ring: 0,
      x: 0.5, y: 0.5, vx: 0, vy: 0,
      card: { title: `${me!.trim()}（我）`, type: "人 · 本地身份", lines: ["尚未加入团队 · 下面是你的本地节点"] },
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
    const isMine = mc.owner ? mc.owner === meName : true; // 无主 = 我的本地节点
    lines.push(mc.owner ? `属于 ${mc.owner}` : "我的本地节点");
    if (via) lines.push(`经门 ${via} 可达`);
    if (mc.owner) lines.push(reachers.length ? `可进 ${reachers.join("、")}` : "无人可进");
    // 可连性:点了能不能真开终端 —— 说清楚,别让点击静默失败。
    if (!mc.connectable) lines.push("⚠ 未在本地拓扑");
    else if (!mc.has_secret) lines.push("⚠ 未配凭据");
    else lines.push("点击打开终端");
    const ring = gate ? RING.gate : isMine ? RING.myMachine : RING.farMachine;
    nodes.push({ id: `m:${mc.name}`, name: mc.name, kind: "machine", compute: true, gate, me: false, ring, ...place(ring), vx: 0, vy: 0, card: { title: disp(mc.name), type: gate ? `门 · ${mc.host}` : `机器 · ${mc.host}`, lines } });
    if (mc.owner) edges.push({ a: `p:${mc.owner}`, b: `m:${mc.name}` }); // 归属(自身设备无此边,已折进人)
    else if (meId) edges.push({ a: meId, b: `m:${mc.name}` });           // 我的本地机 → 连到我
  }
  // 机器 → 它的门(可达路径边:这台机经哪道门进来)。
  for (const mc of servers) {
    const via = gateOf(mc);
    if (via && via !== mc.name) edges.push({ a: `m:${mc.name}`, b: `m:${via}` });
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
  // 去重(归属+门+授权可能产生重复边)。
  const seen = new Set<string>();
  const uniq = edges.filter((e) => {
    const k1 = `${e.a}|${e.b}`, k2 = `${e.b}|${e.a}`;
    if (seen.has(k1) || seen.has(k2)) return false;
    seen.add(k1);
    return true;
  });
  return { nodes, edges: uniq };
}

export function TeamGraph({ view, me, onOpen, active, reach }: {
  view: Fabric;
  me?: string;
  onOpen?: (server: string) => void;
  active?: string[];                 // 有活 SSH 会话的服务器名 → 常亮
  reach?: Record<string, boolean>;   // 探测结果 name→可达(脉冲)/不可达(灰);缺省=未知
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  const hover = useRef<string | null>(null);
  // 回调/状态放 ref:父组件每次渲染都会新建,进 effect 依赖会重建整张图(力导向重新抖)。
  const openRef = useRef(onOpen);
  openRef.current = onOpen;
  const statRef = useRef<{ a: Set<string>; r: Record<string, boolean> }>({ a: new Set(), r: {} });
  statRef.current = { a: new Set(active ?? []), r: reach ?? {} };

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
      live: col("--success"),     // 状态色:已连接/可达
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

    // 节点状态:已连接 > 可达 > 不可达 > 未知。连接键 = n.name(机器名/成员名即 store 名)。
    const stOf = (n: N): "on" | "up" | "down" | null => {
      if (!(n.kind === "machine" || n.compute)) return null;
      const st = statRef.current;
      if (st.a.has(n.name)) return "on";
      const r = st.r[n.name];
      if (r === true) return "up";
      if (r === false) return "down";
      return null;
    };

    const physics = () => {
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
        const f = ((Math.hypot(dx, dy) || 0.001) - 0.15) * 0.02;
        a.vx += dx * f; a.vy += dy * f; b.vx -= dx * f; b.vy -= dy * f;
      }
      for (const n of nodes) {
        if (n.me) { n.x = 0.5; n.y = 0.5; n.vx = 0; n.vy = 0; continue; } // 我 = 固定圆心
        if (meId) {
          // 以我为中心的径向偏好:把节点温和拉向它的关系环(近=我的机器,远=队友的机器)。
          const dx = n.x - 0.5, dy = n.y - 0.5;
          const d = Math.hypot(dx, dy) || 0.001;
          const f = (n.ring - d) * 0.012;
          n.vx += (dx / d) * f; n.vy += (dy / d) * f;
        } else {
          n.vx += (0.5 - n.x) * 0.006; n.vy += (0.5 - n.y) * 0.006; // 无「我」:退回向心网
        }
        n.vx *= 0.85; n.vy *= 0.85;
        n.x += n.vx; n.y += n.vy;
        n.x = Math.max(0.1, Math.min(0.9, n.x));
        n.y = Math.max(0.12, Math.min(0.88, n.y));
      }
    };

    const render = () => {
      const W = canvas.clientWidth, H = canvas.clientHeight;
      const p = pal();
      const ctx = canvas.getContext("2d")!;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, W, H);
      ctx.lineCap = "round";
      const X = (v: number) => v * W, Y = (v: number) => v * H;
      const t = performance.now() / 1000;
      const pulse = 0.5 + 0.5 * Math.sin(t * 2.4); // 可达节点的呼吸相位
      // 节点大小自适应:画布面积 / 节点数 → 密时缩小,疏时放大。
      const base = Math.max(4.5, Math.min(9, Math.sqrt((W * H) / Math.max(1, nodes.length)) * 0.055));
      const hv = hover.current;
      // 焦点:悬停优先;没悬停时「我」是软焦点(默认微微亮出我能进的机器)。
      const focus = hv ?? meId;
      const strong = !!hv;
      const near = focus ? nbr.get(focus) : null;

      // 边(统一素线;轻微弧线,密集时不打架)
      for (const e of edges) {
        const a = nodes[idx.get(e.a)!], b = nodes[idx.get(e.b)!];
        if (!a || !b) continue;
        const on = focus && (a.id === focus || b.id === focus);
        const ax = X(a.x), ay = Y(a.y), bx = X(b.x), by = Y(b.y);
        const dx = bx - ax, dy = by - ay, len = Math.hypot(dx, dy) || 1;
        const cx = (ax + bx) / 2 + (-dy / len) * len * 0.12;
        const cy = (ay + by) / 2 + (dx / len) * len * 0.12;
        ctx.beginPath(); ctx.moveTo(ax, ay); ctx.quadraticCurveTo(cx, cy, bx, by);
        if (on) { ctx.strokeStyle = hexA(p.dotHi, strong ? 0.75 : 0.42); ctx.lineWidth = strong ? 1.9 : 1.5; }
        else if (focus) { ctx.strokeStyle = hexA(p.edge, strong ? 0.1 : 0.42); ctx.lineWidth = 1.1; }
        else { ctx.strokeStyle = hexA(p.edge, 0.5); ctx.lineWidth = 1.2; }
        ctx.stroke();
      }

      // 节点
      for (const n of nodes) {
        const x = X(n.x), y = Y(n.y);
        const isFocus = focus === n.id, isNb = !!near?.has(n.id);
        const dim = strong && !isFocus && !isNb;
        const r = isFocus ? base * 1.45 : n.me ? base * 1.2 : base;
        const stat = stOf(n);
        // 「我」环:始终在点外描一圈主色,一眼找到自己。
        if (n.me) {
          ctx.beginPath(); ctx.arc(x, y, r + 4, 0, Math.PI * 2);
          ctx.strokeStyle = hexA(p.dotHi, dim ? 0.3 : 0.9); ctx.lineWidth = 1.6; ctx.stroke();
        }
        const base2 = n.kind === "person" ? p.person : p.machine;
        // 「人+算力」环:人节点若共享了自身设备,外描一圈算力色。
        if (n.kind === "person" && n.compute) {
          ctx.beginPath(); ctx.arc(x, y, r + 2.4, 0, Math.PI * 2);
          ctx.strokeStyle = hexA(p.machine, dim ? 0.3 : 0.85); ctx.lineWidth = 2; ctx.stroke();
        }
        // 「门」标记:主色菱形 = 一道进内网的门。
        if (n.gate) {
          const gr = r + 5;
          ctx.beginPath();
          ctx.moveTo(x, y - gr); ctx.lineTo(x + gr, y); ctx.lineTo(x, y + gr); ctx.lineTo(x - gr, y); ctx.closePath();
          ctx.strokeStyle = hexA(p.dotHi, dim ? 0.3 : 0.9); ctx.lineWidth = 1.6; ctx.stroke();
        }
        // ── 状态皮肤 ──
        // 已连接:常亮 —— 稳定光环 + 轻辉光。可达:呼吸脉冲环。不可达:灰点。
        if (stat === "on" && !dim) {
          ctx.beginPath(); ctx.arc(x, y, r + 3.2, 0, Math.PI * 2);
          ctx.strokeStyle = hexA(p.live, 0.9); ctx.lineWidth = 2; ctx.stroke();
          ctx.shadowColor = hexA(p.live, 0.55); ctx.shadowBlur = 10;
        } else if (stat === "up" && !dim) {
          ctx.beginPath(); ctx.arc(x, y, r + 2 + pulse * 3.4, 0, Math.PI * 2);
          ctx.strokeStyle = hexA(p.live, 0.15 + (1 - pulse) * 0.6); ctx.lineWidth = 1.6; ctx.stroke();
        }
        if (isFocus && strong) { ctx.shadowColor = hexA(base2, 0.65); ctx.shadowBlur = 12; }
        ctx.beginPath(); ctx.arc(x, y, r, 0, Math.PI * 2);
        const fill = stat === "down" ? hexA(p.faint, dim ? 0.22 : 0.55) : dim ? hexA(base2, 0.28) : base2;
        ctx.fillStyle = fill;
        ctx.fill(); ctx.shadowBlur = 0;
        // 名字标签
        const showLabel = !(isFocus && strong) && !dim && !(strong && !isNb && !isFocus);
        if (showLabel) {
          ctx.fillStyle = n.me ? hexA(p.dotHi, 0.95) : stat === "down" ? hexA(p.faint, 0.6) : p.faint;
          ctx.font = `${n.me ? "600" : "500"} ${Math.round(Math.max(9, Math.min(11, base + 3)))}px system-ui, sans-serif`;
          ctx.textAlign = "center"; ctx.textBaseline = "top";
          ctx.fillText(clip(disp(n.name), 10), x, y + r + 3);
        }
      }

      // 悬停详情卡(最上层)
      if (hv) {
        const n = nodes[idx.get(hv)!];
        if (n) drawCard(ctx, X(n.x), Y(n.y), W, H, n.card, p);
      }
      return base;
    };

    let hitR = 14; // 命中半径(像素),随自适应节点大小更新
    const step = () => {
      if (settle < 300) { physics(); settle++; }
      const b = render(); // 状态脉冲需要持续重绘;物理只在松弛期积分,之后每帧仅重绘
      if (b) hitR = b * 2.8;
      raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);

    // 命中测试:光标 → 最近的节点(像素域,随节点大小自适应)。悬停与点击共用 ——
    // 点击**不能**依赖 hover 状态,否则触屏(无 hover)和合成点击都点不动。
    const hit = (ev: MouseEvent): N | null => {
      const rect = canvas.getBoundingClientRect();
      const mx = ev.clientX - rect.left, my = ev.clientY - rect.top;
      let best: N | null = null, bd = hitR;
      for (const n of nodes) {
        const d = Math.hypot(n.x * rect.width - mx, n.y * rect.height - my);
        if (d < bd) { bd = d; best = n; }
      }
      return best;
    };
    const onMove = (ev: MouseEvent) => {
      const id = hit(ev)?.id ?? null;
      if (id !== hover.current) hover.current = id;
      canvas.style.cursor = id ? "pointer" : "default";
    };
    canvas.addEventListener("mousemove", onMove);
    // 点节点 → 开终端。图是**启动器**不是只读画:地图选位置,工作面出终端。
    const onClick = (ev: MouseEvent) => {
      const n = hit(ev);
      if (!n) return;
      if (n.kind === "machine" || n.compute) openRef.current?.(n.name); // 纯消费的人节点不可连
    };
    canvas.addEventListener("click", onClick);
    const onLeave = () => { hover.current = null; };
    canvas.addEventListener("mouseleave", onLeave);
    const onResize = () => { resize(); settle = Math.min(settle, 260); };
    window.addEventListener("resize", onResize);

    return () => { cancelAnimationFrame(raf); canvas.removeEventListener("mousemove", onMove); canvas.removeEventListener("click", onClick); canvas.removeEventListener("mouseleave", onLeave); window.removeEventListener("resize", onResize); };
  }, [view, me]);

  return (
    <div className="tgraph">
      <canvas ref={ref} style={{ height }} />
      <div className="tgraph-legend">
        <span><i className="d person" />人</span>
        <span><i className="d compute" />人+算力</span>
        <span><i className="d machine" />机器</span>
        <span><svg className="d-diamond" width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"><polygon points="6,1 11,6 6,11 1,6" fill="none" stroke="var(--accent)" strokeWidth={2} /></svg>门</span>
        <span><i className="d me" />我(圆心)</span>
        <span className="sep" />
        <span><i className="d st-on" />已连接</span>
        <span><i className="d st-up" />可达</span>
        <span><i className="d st-down" />不可达</span>
        <span className="hint">悬停查看 · 点节点开终端</span>
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
