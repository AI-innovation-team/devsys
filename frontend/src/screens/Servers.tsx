import { Me, Server } from "../api";
import { Icon } from "../icons";

export function Servers({ me, goSettings }: { me: Me | null; goSettings: () => void }) {
  const servers = me?.servers || [];
  const ready = servers.filter((s) => s.has_secret && s.username).length;
  return (
    <div className="wrap">
      <header className="page-head">
        <div className="eyebrow">developer gateway</div>
        <h1>服务器</h1>
        <p>选择目标机，从浏览器打开 Web SSH 或 VS Code。连接始终以你自己的身份进行。</p>
        <div className="stats">
          <div className="stat"><div className="v">{servers.length}</div><div className="l">Servers</div></div>
          <div className="stat"><div className="v">{ready}</div><div className="l">Ready</div></div>
          <div className="stat"><div className="v">{servers.length - ready}</div><div className="l">Unset</div></div>
        </div>
      </header>
      <div className="cards">
        {servers.map((s) => <LaunchCard key={s.name} s={s} goSettings={goSettings} />)}
      </div>
    </div>
  );
}

function LaunchCard({ s, goSettings }: { s: Server; goSettings: () => void }) {
  const ready = !!(s.has_secret && s.username);
  const action = (label: string, icon: string, url: string, cls: string) =>
    ready ? (
      <a className={"btn " + cls} href={url} target="_blank" rel="noreferrer"><Icon name={icon} />{label}</a>
    ) : (
      <button className={"btn " + cls} disabled title="请先在设置中配置凭据"><Icon name={icon} />{label}</button>
    );
  return (
    <article className="card">
      <div className="card-head">
        <div>
          <div className="srv-title">
            <span className={"srv-dot" + (ready ? " ok" : "")} />
            <span className="srv-name">{s.name}</span>
            {s.jump ? <span className="badge">via {s.jump}</span> : <span className="badge info">内网</span>}
          </div>
          <div className="srv-host"><Icon name="network" />{s.host}:{s.port}</div>
        </div>
        <div className="srv-actions">
          {action("SSH", "terminal", `/terminal/${encodeURIComponent(s.name)}`, "secondary")}
          {action("VS Code", "code", `/vscode/${encodeURIComponent(s.name)}`, "primary")}
        </div>
      </div>
      {!ready && (
        <div className="cred">
          <button className="cred-toggle" onClick={goSettings}>
            <span className="badge warn"><Icon name="alert" />未设置凭据</span>
            <span className="cred-hint">前往设置<Icon name="chevron" className="chev" /></span>
          </button>
        </div>
      )}
    </article>
  );
}
