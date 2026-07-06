# 证书与网络

## TLS 证书

Caddy 通过 DNS-01 质询自动签发并续期证书(阿里云 DNS),无需暴露 80 端口。
凭据放在 `.env`,渲染时注入,不入仓库。

## 国内网络

- oauth2-proxy 与 GitHub 换 token 走本地代理(mihomo,`HTTPS_PROXY=127.0.0.1:7890`)。
- GitHub OAuth 对国内不够稳定时,引导用户改用**邮箱登录**。

## 反向隧道

gateway 用一把独立密钥向 relay 建立反向隧道;relay 侧用 `GatewayPorts` 放行,
公钥经 `authorize.sh` 登记,互不写死、可泛化到多网关。
