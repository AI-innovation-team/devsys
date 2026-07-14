import { useEffect, useState } from "react";

import { data, type ProvisionPlan, type ProvisionResult } from "../data";
import { Icon } from "../icons";

const TIER_LABEL: Record<number, string> = { 0: "纯跳板", 1: "受限计算", 2: "完全信任" };

// 授权下发：把团队成员的公钥真正装进这台被共享机（让队友能登进去）。
// 铁律：先给人看脚本，再执行 —— 绝不静默改机器（责任为门：主人自己点头）。
export function ProvisionModal({
  teamPath,
  server,
  tier,
  onClose,
}: {
  teamPath: string;
  server: string;
  tier: number;
  onClose: () => void;
}) {
  const [plan, setPlan] = useState<ProvisionPlan | null>(null);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ProvisionResult | null>(null);
  const [showScript, setShowScript] = useState(false);

  useEffect(() => {
    data.provisionPreview(teamPath, server, tier)
      .then(setPlan)
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)));
  }, [teamPath, server, tier]);

  const apply = async () => {
    setBusy(true);
    setErr("");
    try {
      setResult(await data.provisionApply(teamPath, server, tier));
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
            <div className="prov-subt">档 {tier} · {TIER_LABEL[tier] ?? "?"}</div>
          </div>
          <button className="btn subtle sm" onClick={onClose}><Icon name="x" /></button>
        </div>

        <div className="prov-body">
          {err && <div className="import-err">{err}</div>}

          {plan && !result && (
            <>
              <p className="prov-intro">
                将在 <strong>{server}</strong> 上为每位团队成员建立<strong>独立账号</strong>并装入其公钥
                —— 身份到人，不用共享账号。凭据不出本机，装的只是公钥。
              </p>

              {plan.accounts.length > 0 ? (
                <div className="prov-accts">
                  <div className="prov-sec-t">将建立 / 更新的账号</div>
                  <div className="prov-chips">
                    {plan.accounts.map((a) => (
                      <span key={a} className={"prov-chip" + (plan.sudo ? " sudo" : "")}>
                        <Icon name="user" />{a}{plan.sudo && <em>sudo</em>}
                      </span>
                    ))}
                  </div>
                </div>
              ) : (
                <div className="import-err">没有可下发的成员公钥 —— 队友仍然登不进来。</div>
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
              <span className="save-note">需要这台机的凭据具备 root / 免密 sudo</span>
            </>
          ) : (
            <button className="btn secondary sm" onClick={onClose}>关闭</button>
          )}
        </div>
      </div>
    </div>
  );
}
