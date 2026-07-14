import { useState } from "react";

import type { SshHost } from "../data";
import { Icon } from "../icons";

// 从 ~/.ssh/config 解析出的主机列表，勾选后导入。
export function ImportModal({
  path,
  hosts,
  existing,
  onPick,
  onImport,
  onCancel,
}: {
  path: string;
  hosts: SshHost[];
  existing: Set<string>;
  onPick: () => Promise<void>;
  onImport: (selected: SshHost[]) => Promise<void>;
  onCancel: () => void;
}) {
  // 默认勾选尚未存在的主机
  const [sel, setSel] = useState<Set<string>>(
    () => new Set(hosts.filter((h) => !existing.has(h.name)).map((h) => h.name)),
  );
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  const toggle = (name: string) =>
    setSel((s) => {
      const n = new Set(s);
      n.has(name) ? n.delete(name) : n.add(name);
      return n;
    });

  const doImport = async () => {
    const chosen = hosts.filter((h) => sel.has(h.name));
    if (!chosen.length) { setErr("未勾选任何主机"); return; }
    setBusy(true);
    setErr("");
    try {
      await onImport(chosen);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal modal-wide" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title"><Icon name="server" />导入 SSH 主机</div>
        <div className="import-path">
          <span className="import-path-txt" title={path}>{path}</span>
          <button className="btn subtle sm" onClick={onPick}><Icon name="folder" />选择文件</button>
        </div>
        <p className="modal-desc">勾选要导入的主机，带 key 的连私钥一起导入。</p>

        <div className="import-list">
          {hosts.map((h) => {
            const dup = existing.has(h.name);
            return (
              <label key={h.name} className="import-row">
                <input type="checkbox" checked={sel.has(h.name)} onChange={() => toggle(h.name)} />
                <div className="import-main">
                  <div className="import-name">
                    {h.name}
                    {h.jump && <span className="badge">via {h.jump}</span>}
                    {h.auth === "key" && <span className="badge">key</span>}
                    {dup && <span className="badge warn">已存在</span>}
                  </div>
                  <div className="import-sub">
                    {(h.username ? h.username + "@" : "") + h.host + ":" + h.port}
                  </div>
                </div>
              </label>
            );
          })}
        </div>

        {err && <div className="modal-err">{err}</div>}
        <div className="modal-actions">
          <button className="btn subtle sm" onClick={onCancel}>取消</button>
          <button className="btn primary sm" disabled={busy} onClick={doImport}>
            导入选中 ({sel.size})
          </button>
        </div>
      </div>
    </div>
  );
}
