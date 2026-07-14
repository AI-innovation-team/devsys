import { useState } from "react";

import { data } from "../data";
import { Icon } from "../icons";

// 「连接团队」。v1 第一刀:从本地 team.yaml 加载团队共享机(作为只读节点合并进列表)。
// 心智:团队 = 一份共享配置 + 每人登记自己的设备,协调器极轻。共享拓扑不共享凭据,
// 连接后你用自己的凭据连。授权/tailnet/贡献侧后续再叠(见共享算力模型)。
export function Team({ reload, goServers }: { reload: () => Promise<void> | void; goServers: () => void }) {
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [done, setDone] = useState<{ team: string; added: number; skipped: string[] } | null>(null);

  const pick = async () => {
    setErr("");
    const p = await data.pickTeamFile();
    if (p) { setPath(p); setDone(null); }
  };

  const load = async () => {
    if (!path) { setErr("先选择一个 team.yaml 文件"); return; }
    setBusy(true);
    setErr("");
    try {
      const r = await data.loadTeam(path);
      await reload();
      setDone({ team: r.team, added: r.added, skipped: r.skipped });
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="wrap">
      <header className="page-head">
        <h1>连接团队</h1>
        <p>团队是一份共享配置 + 每人登记自己的设备。加载 team.yaml 后，团队共享的服务器会作为<strong>只读节点</strong>出现在你的列表里，你始终用自己的凭据连接。</p>
      </header>

      <article className="card open">
        <div className="cfg-head">
          <div className="srv-title"><span className="srv-name" style={{ fontSize: 17 }}>从 team.yaml 加载</span></div>
        </div>
        <div className="cfg-body">
          <div className="field">
            <label>团队配置文件</label>
            <div className="inp" style={{ cursor: "pointer" }} onClick={pick}>
              <Icon name="folder" />
              <input value={path} placeholder="选择一个 team.yaml…" readOnly style={{ cursor: "pointer" }} />
            </div>
          </div>
          {err && <div className="import-err">{err}</div>}
          {done && (
            <div className="team-done">
              <Icon name="check" />
              <span>
                已加载团队 <strong>{done.team}</strong>：新增 {done.added} 台
                {done.skipped.length > 0 && `，跳过 ${done.skipped.length} 台同名（${done.skipped.join("、")}）`}
              </span>
            </div>
          )}
          <div className="cred-foot">
            <button className="btn primary sm" disabled={busy || !path} onClick={load}>
              <Icon name="users" />{busy ? "加载中…" : "加载"}
            </button>
            <button className="btn subtle sm" onClick={pick}>选择文件</button>
            {done && <button className="btn subtle sm" onClick={goServers}>去服务器页看</button>}
          </div>
        </div>
      </article>

      <div className="team-why">
        <div className="team-why-head"><Icon name="network" /><span>它是怎么运作的</span></div>
        <ul>
          <li><strong>配置即代码</strong> · 团队约定一份 <code>team.yaml</code>，每人往里登记自己的设备与授权，协调器轻到几乎没有。</li>
          <li><strong>数据面复用原生 SSH</strong> · 团队只给你「拓扑」，连接仍是你的 app 直连对方内网，凭据不出本机。</li>
          <li><strong>只有节点，没有中心</strong> · 你加的机器与团队给的机器共用一张列表、一套策略，只是来源不同。</li>
        </ul>
      </div>
    </div>
  );
}
