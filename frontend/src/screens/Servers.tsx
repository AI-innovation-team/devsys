import { Me, Server } from "../api";
import { Icon } from "../icons";

export function Servers({ me, goSettings }: { me: Me | null; goSettings: () => void }) {
  const servers = me?.servers || [];
  return (
    <div className="wrap">
      <header className="page-head"><h1>服务器</h1></header>
      <div className="cards">
        {servers.map((s) => <LaunchCard key={s.name} s={s} goSettings={goSettings} />)}
      </div>
    </div>
  );
}

function LaunchCard({ s, goSettings }: { s: Server; goSettings: () => void }) {
  const ready = !!(s.has_secret && s.username);
  return (
    <article className="card">
      <div className="card-head">
        <div>
          <div className="srv-title">
            <span className={"srv-dot" + (ready ? " ok" : "")} />
            <span className="srv-name">{s.name}</span>
            {s.jump && <span className="badge">via {s.jump}</span>}
          </div>
          <div className="srv-host"><Icon name="network" />{s.host}:{s.port}</div>
        </div>
        <div className="srv-actions">
          {ready ? (
            <>
              <a className="btn secondary" href={`/terminal/${encodeURIComponent(s.name)}`} target="_blank" rel="noreferrer"><Icon name="terminal" />SSH</a>
              <a className="btn primary" href={`/vscode/${encodeURIComponent(s.name)}`} target="_blank" rel="noreferrer"><Icon name="code" />VS Code</a>
            </>
          ) : (
            <button className="btn subtle" onClick={goSettings}><Icon name="settings" />设置凭据</button>
          )}
        </div>
      </div>
    </article>
  );
}
