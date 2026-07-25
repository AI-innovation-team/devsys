import { useEffect, useMemo, useState } from "react";

import { TeamGraph } from "../components/TeamGraph";
import { data, toFabric, type Fabric, type TeamView } from "../data";
import type { Server } from "../api";

// 主页 = 织物图这张「地图」：谁在网上、够得着谁。点一个节点 → 跳到「工作区」开出它的终端。
// 图是启动器/导航器,不是只读画；工作面(活终端)在侧栏「工作区」里(常驻,切走不断线)。
export function Home({
  teamPath,
  servers,
  onOpen,
}: {
  teamPath: string;
  servers: Server[];
  onOpen: (name: string) => void; // 校验通过 → 交给 App 开 pane 并切到工作区
}) {
  const [team, setTeam] = useState<TeamView | null>(null);
  const [ident, setIdent] = useState("");
  const [note, setNote] = useState("");
  // 节点状态:已连接(常亮)= 活 SSH 会话;可达(脉冲)/不可达(灰)= 周期 TCP 探测。
  const [active, setActive] = useState<string[]>([]);
  const [reach, setReach] = useState<Record<string, boolean>>({});

  useEffect(() => {
    let stop = false;
    let un: (() => void) | undefined;
    data.sshActive().then((a) => { if (!stop) setActive(a); }).catch(() => {});
    // 会话开/关时后端推送最新活跃集,图即时点亮/熄灭。
    const w = window as unknown as { __TAURI__?: any };
    w.__TAURI__?.event?.listen?.("ssh://active", (ev: { payload: string[] }) => setActive(ev.payload ?? []))
      .then((f: () => void) => { un = f; });
    // 本机恒可达(它就是这台机),探测结果上直接盖上。
    const probe = () => data.probeReach().then((r) => { if (!stop) setReach({ ...r, "~local": true }); }).catch(() => {});
    probe();
    const t = window.setInterval(probe, 25000);
    return () => { stop = true; window.clearInterval(t); un?.(); };
  }, []);

  useEffect(() => {
    // 「我」的身份锚:GitHub 登录名优先(团队成员就是 GitHub login,别的名字匹配不上),
    // 未登录 GitHub 再退到 tsnet/系统 tailscale/OS 用户。
    data.ghState().then((s) => {
      if (s.login) { setIdent(s.login); return; }
      return data.localIdentity().then((i) => setIdent(i.login));
    }).catch(() => { data.localIdentity().then((i) => setIdent(i.login)).catch(() => {}); });
  }, []);

  useEffect(() => {
    if (!teamPath) { setTeam(null); return; }
    data.readTeamView(teamPath).then(setTeam).catch(() => setTeam(null));
  }, [teamPath]);

  // 统一织物：本地自持服务器 ∪ 团队共享机（按 name 合并）。没有团队也能用（只画本地机）。
  const fabric: Fabric = useMemo(() => toFabric(servers, team), [servers, team]);

  const open = (name: string) => {
    const m = fabric.machines.find((x) => x.name === name);
    // 点击不静默失败：连不上时说清楚为什么。
    // 团队机现在激活时自动并入本地拓扑;还连不上多半是与本地服务器重名被跳过。
    if (m && !m.connectable) { setNote(`「${name}」未并入本地拓扑(可能与本地服务器重名被跳过)—— 去「连接团队」页看加载结果。`); return; }
    if (m && !m.has_secret) { setNote(`「${name}」还没配凭据 —— 去「服务器」页给它填密码/私钥。`); return; }
    setNote("");
    onOpen(name); // 交给 App：开 pane + 切到「工作区」视图
  };

  return (
    <div className="cockpit solo">
      <div className="cockpit-graph">
        <div className="cockpit-head">
          <h2>织物</h2>
          <span className="hint">
            {fabric.machines.length} 个算力节点
            {fabric.members.length ? ` · ${fabric.members.length} 人` : ""}
            {fabric.team ? ` · ${fabric.team}` : " · 未连团队"}
            {" · 点节点在「工作区」开终端"}
          </span>
        </div>
        {fabric.machines.length || fabric.members.length ? (
          <TeamGraph view={fabric} me={ident} onOpen={open} active={active} reach={reach} />
        ) : (
          <p className="cockpit-blank">还没有节点。去「服务器」加一台机，或在「连接团队」加载 team.yaml。</p>
        )}
        {note && <p className="cockpit-note">{note}</p>}
      </div>
    </div>
  );
}
