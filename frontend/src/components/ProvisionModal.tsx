import { useEffect, useState } from "react";

import { data, type ProvisionPlan, type ProvisionResult } from "../data";
import { Icon } from "../icons";

// 授权下发：把团队成员的公钥真正装进这台被共享机（让队友能登进去）。
// 每人的权限按其角色 × 该机 grant 算出（RBAC）—— 同机可以有人 sudo、有人受限。
// 铁律：先给人看脚本，再执行 —— 绝不静默改机器（责任为门：主人自己点头）。
export function ProvisionModal({
  teamPath,
  server,
  onClose,
}: {
  teamPath: string;
  server: string;
  onClose: () => void;
}) {
  const [plan, setPlan] = useState<ProvisionPlan | null>(null);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ProvisionResult | null>(null);
  const [showScript, setShowScript] = useState(false);

  useEffect(() => {
    data.provisionPreview(teamPath, server)
      .then(setPlan)
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)));
  }, [teamPath, server]);

  const apply = async () => {
    setBusy(true);
    setErr("");
    try {
      setResult(await data.provisionApply(teamPath, server));
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="prov-modal" onClick={(e) => e.stopPropagation()}>
        <div className="prov-head">
          <div>
            <div className="prov-title">授权下发 · {server}</div>
            <div className="prov-subt">
              按角色分配权限（RBAC）·{" "}
              {plan?.isolation === "rootless" ? "一人一容器 · 免 root"
                : plan?.isolation === "container" ? "一人一容器" : "裸机账号"}
            </div>
          </div>
          <button className="btn subtle sm" onClick={onClose}><Icon name="x" /></button>
        </div>

        <div className="prov-body">
          {err && <div className="import-err">{err}</div>}

          {plan && !result && (
            <>
              <p className="prov-intro">
                {plan.isolation === "rootless" ? (
                  <>将在 <strong>{server}</strong> 上给每位有权成员建<strong>一个独立容器</strong>，
                  <strong>全程用你自己的普通账号</strong> —— 不建宿主账号、不碰 <code>/etc</code>、不需要 sudo。
                  队友直连他自己容器里的 sshd（各占一个高位端口，见下）。</>
                ) : plan.isolation === "container" ? (
                  <>将在 <strong>{server}</strong> 上给每位有权成员建<strong>一个独立容器</strong>，
                  并装入其公钥 —— 身份到人、容器到人。他 SSH 进来直接落进自己的容器，拿不到宿主 shell。
                  凭据不出本机，装的只是公钥。</>
                ) : (
                  <>将在 <strong>{server}</strong> 上为每位有权成员建立<strong>独立账号</strong>并装入其公钥
                  —— 身份到人，权限按角色定（core 拿 sudo、member 受限）。凭据不出本机，装的只是公钥。</>
                )}
              </p>

              {plan.isolation !== "account" && (
                <div className="prov-accts">
                  <div className="prov-sec-t">
                    {plan.isolation === "rootless" ? "逐容器限额" : "借出资源池（父 cgroup）"}
                  </div>
                  <div className="prov-pool">
                    {!plan.pool ? <>未设上限 —— 容器不限 CPU/内存。</>
                      : plan.isolation === "rootless"
                        ? <>每个容器各自上限 <strong>{plan.pool}</strong> —— 免 root 下没有父池，N 个人最多占到 N 倍。</>
                        : <>所有借用容器挂在同一个池下，加起来永不超 <strong>{plan.pool}</strong>。</>}
                  </div>
                  {plan.datasets.length > 0 && (
                    <>
                      <div className="prov-sec-t">挂进每个容器的共享数据集</div>
                      <div className="prov-chips">
                        {plan.datasets.map((d) => (
                          <span key={d} className="prov-chip"><Icon name="server" />{d}</span>
                        ))}
                      </div>
                    </>
                  )}
                </div>
              )}

              {plan.accounts.length > 0 ? (
                <div className="prov-accts">
                  <div className="prov-sec-t">
                    {plan.isolation !== "account" ? "将建立 / 更新的容器与账号" : "将建立 / 更新的账号"}
                  </div>
                  <div className="prov-chips">
                    {plan.accounts.map((a) => (
                      <span key={a.name} className={"prov-chip" + (a.sudo ? " sudo" : "")}>
                        <Icon name="user" />{a.name}
                        <em>
                          {a.role}
                          {a.mode === "forward" && " · 仅借道"}
                          {a.mode === "container" && ` · ${a.container}`}
                          {!!a.port && ` · :${a.port}`}
                          {a.sudo && " · sudo"}
                        </em>
                      </span>
                    ))}
                  </div>
                </div>
              ) : (
                <div className="import-err">没有可下发的账号 —— 无成员获此机授权，或成员缺公钥。</div>
              )}

              {plan.warnings.map((w, i) => (
                <div key={i} className="acl-note"><Icon name="alert" /><span>{w}</span></div>
              ))}

              <button className="prov-toggle" onClick={() => setShowScript((s) => !s)}>
                <Icon name="chevron" className={showScript ? "flip" : ""} />
                {showScript ? "收起脚本" : "查看将执行的脚本"}
              </button>
              {showScript && <pre className="acl-json">{plan.script}</pre>}
            </>
          )}

          {result && (
            <>
              <div className={result.ok ? "team-done" : "import-err"}>
                <Icon name={result.ok ? "check" : "alert"} />
                <span>{result.ok ? "下发成功 —— 队友现在可以用自己的账号登入。" : `下发失败（退出码 ${result.code}）`}</span>
              </div>
              <div className="prov-sec-t">执行输出</div>
              <pre className="acl-json">{result.output || "（无输出）"}</pre>
            </>
          )}
        </div>

        <div className="prov-foot">
          {!result ? (
            <>
              <button
                className="btn primary sm"
                disabled={busy || !plan || plan.accounts.length === 0}
                onClick={apply}
              >
                <Icon name="key" />{busy ? "下发中…" : "在这台机上执行"}
              </button>
              <button className="btn subtle sm" onClick={onClose}>取消</button>
              <span className="save-note">
                {plan?.isolation === "rootless"
                  ? "用你自己的普通账号执行 —— 不需要 root"
                  : "需要这台机的凭据具备 root / 免密 sudo"}
              </span>
            </>
          ) : (
            <button className="btn secondary sm" onClick={onClose}>关闭</button>
          )}
        </div>
      </div>
    </div>
  );
}
