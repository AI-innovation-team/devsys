import { Me } from "../api";

// 管理员界面（仅 config.yaml oauth.admins 可见）。
// 阶段 0：骨架。后续阶段接入：用户/凭据总览、邮箱用户管理、服务器增删改、GitHub 白名单、日志。
export function Admin({ me }: { me: Me | null }) {
  return (
    <div className="wrap">
      <header className="page-head">
        <h1>管理</h1>
        <p style={{ margin: "4px 0 0", color: "var(--text-faint)", fontSize: 14 }}>
          服务器 · 用户 · 权限 · 日志　（管理员：{me?.user}）
        </p>
      </header>
      <section className="set-sec">
        <div className="set-h"><h2>即将上线</h2></div>
        <p style={{ color: "var(--text-faint)", fontSize: 14, lineHeight: 1.6 }}>
          管理功能正在分阶段接入：用户/凭据总览、邮箱用户管理、服务器增删改、GitHub 白名单、日志查看。
        </p>
      </section>
    </div>
  );
}
