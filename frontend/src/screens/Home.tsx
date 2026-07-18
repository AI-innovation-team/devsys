import { useEffect, useMemo, useRef, useState } from "react";

import { TeamGraph } from "../components/TeamGraph";
import { data, toFabric, type Fabric, type TeamView } from "../data";
import type { Server } from "../api";
import { Workspace, type WorkspaceHandle } from "./Workspace";

// 主页 = 驾驶舱。左「织物图」当地图/导航（在哪、够得着谁），右「工作区」当工作面
// （正在干什么）；点节点 → 右侧开出它的终端 pane。图是启动器，不是只读画。
export function Home({ teamPath, servers }: { teamPath: string; servers: Server[] }) {
  const [team, setTeam] = useState<TeamView | null>(null);
  const [ident, setIdent] = useState("");
  const [note, setNote] = useState("");
  const wsp = useRef<WorkspaceHandle>(null);

  useEffect(() => {
    data.localIdentity().then((i) => setIdent(i.login)).catch(() => {});
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
    if (m && !m.connectable) { setNote(`「${name}」还不在本地拓扑 —— 先去「连接团队」加载一次 team.yaml。`); return; }
    if (m && !m.has_secret) { setNote(`「${name}」还没配凭据 —— 去「服务器」页给它填密码/私钥。`); return; }
    setNote("");
    wsp.current?.openPane(name);
  };

  return (
    <div className="cockpit">
      <div className="cockpit-graph">
        <div className="cockpit-head">
          <h2>织物</h2>
          <span className="hint">
            {fabric.machines.length} 个算力节点
            {fabric.members.length ? ` · ${fabric.members.length} 人` : ""}
            {fabric.team ? ` · ${fabric.team}` : " · 未连团队"}
          </span>
        </div>
        {fabric.machines.length || fabric.members.length ? (
          <TeamGraph view={fabric} me={ident} onOpen={open} />
        ) : (
          <p className="cockpit-blank">还没有节点。去「服务器」加一台机，或在「连接团队」加载 team.yaml。</p>
        )}
        {note && <p className="cockpit-note">{note}</p>}
      </div>
      <div className="cockpit-workspace">
        <Workspace ref={wsp} />
      </div>
    </div>
  );
}
