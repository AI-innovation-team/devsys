import { useEffect, useState } from "react";

import { data, type SelfNode } from "../data";
import { Icon } from "../icons";

const KIND_LABEL: Record<string, string> = { tailnet: "Tailnet", lan: "局域网", hostname: "主机名" };

// 把「本机」登记成一个节点。
// 心智：没有「本地/远程」之分，只有节点 —— 你这台机器也在织物上，可以贡献给团队：
//   · 它本身就是算力（队友登进来跑计算）
//   · 或它是通往你内网的跳板（队友经它到达你的计算设备）
// 前提是它得能被队友连上：跑着 sshd + 有个够得着的地址。两点都如实告知，不假装。
export function SelfNodeModal({
  existing,
  onCancel,
  onAdded,
}: {
  existing: Set<string>;
  onCancel: () => void;
  onAdded: () => void | Promise<void>;
}) {
  const [self, setSelf] = useState<SelfNode | null>(null);
  const [name, setName] = useState("");
  const [addr, setAddr] = useState("");
  const [username, setUsername] = useState("");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    data.detectSelf()
      .then((s) => {
        setSelf(s);
        setName(existing.has(s.hostname) ? `${s.hostname}-self` : s.hostname);
        setAddr(s.addrs[0]?.value || "");
        setUsername(s.username);
      })
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const add = async () => {
    if (!name.trim() || !addr.trim()) { setErr("名称与地址必填"); return; }
    setBusy(true); setErr("");
    try {
      await data.upsertServer({
        name: name.trim(),
        host: addr.trim(),
        port: 22,
        transport: addr.startsWith("100.") ? "tailnet" : "direct",
        jump: null,
        username: username.trim(),
        auth: "key",
      });
      await onAdded();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="prov-modal" onClick={(e) => e.stopPropagation()}>
        <div className="prov-head">
          <div>
            <div className="prov-title">添加本机</div>
            <div className="prov-subt">你这台机器也是织物上的一个节点</div>
          </div>
          <button className="btn subtle sm" onClick={onCancel}><Icon name="x" /></button>
        </div>

        <div className="prov-body">
          {err && <div className="import-err">{err}</div>}
          {!self ? (
            <p className="prov-intro">探测中…</p>
          ) : (
            <>
              <p className="prov-intro">
                本机可以作为<strong>算力</strong>贡献给团队（队友登进来跑计算），
                也可以作为<strong>跳板</strong>（队友经它到达你的内网机器）。
                加进列表后，就能像别的节点一样共享、下发授权。
              </p>

              {!self.sshd && (
                <div className="acl-note">
                  <Icon name="alert" />
                  <span>
                    本机<strong>没有在监听 SSH（22）</strong> —— 队友现在连不进来。
                    macOS：系统设置 → 通用 → 共享 → 打开「远程登录」；Linux：启用 sshd。
                    （仍可先加进来，等开了 sshd 就能用。）
                  </span>
                </div>
              )}

              <div className="row2" style={{ marginTop: 14 }}>
                <div className="field">
                  <label>节点名</label>
                  <div className="inp"><Icon name="server" />
                    <input value={name} onChange={(e) => setName(e.target.value)} autoComplete="off" />
                  </div>
                </div>
                <div className="field">
                  <label>用户名（你在本机的账号）</label>
                  <div className="inp"><Icon name="user" />
                    <input value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="off" />
                  </div>
                </div>
              </div>

              <div className="prov-sec-t">队友该用哪个地址找到你</div>
              <div className="tier-pick">
                {self.addrs.map((a) => (
                  <button
                    key={a.value}
                    className={"tier-opt" + (addr === a.value ? " on" : "")}
                    onClick={() => setAddr(a.value)}
                  >
                    <span className="tier-l">
                      {a.value}
                      <span className={"addr-kind " + a.kind}>{KIND_LABEL[a.kind] ?? a.kind}</span>
                    </span>
                    <span className="tier-h">{a.hint}</span>
                  </button>
                ))}
              </div>

              {self.notes.filter((n) => !n.includes("跳板")).map((n, i) => (
                <div key={i} className="acl-note" style={{ marginTop: 10 }}>
                  <Icon name="alert" /><span>{n}</span>
                </div>
              ))}
            </>
          )}
        </div>

        <div className="prov-foot">
          <button className="btn primary sm" disabled={busy || !self || !addr} onClick={add}>
            <Icon name="plus" />{busy ? "添加中…" : "添加为节点"}
          </button>
          <button className="btn subtle sm" onClick={onCancel}>取消</button>
          <span className="save-note">之后在卡片上点 👥 即可共享给团队</span>
        </div>
      </div>
    </div>
  );
}
