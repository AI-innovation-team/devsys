# 架构

一次请求的路径:

```
浏览器
  → 公网 relay(反向隧道入口)
  → 内网 gateway:Caddy → oauth2-proxy(鉴权)→ 门户后端(FastAPI)
  → 目标服务器(以用户身份 SSH / code-server / tmux)
```

## 角色

- **relay**:公网 ECS,只做轻量转发,不碰业务。
- **gateway**:内网网关,承载鉴权与门户,重活都在这里。
- **node**:目标机,门户不持有 root,以每个用户自己的账号连接。

## 鉴权

Caddy 把请求交给 oauth2-proxy;通过后注入 `X-Auth-Request-User` 头,
门户据此识别当前用户,再取该用户加密存储的连接凭据发起连接。
