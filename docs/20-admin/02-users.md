# 用户管理

门户为**申请制**,两种身份:GitHub 白名单、邮箱 + 密码。

## 邮箱账号

在网关机上运行:

```bash
bash ~/gateway/add-email-user.sh alice@example.com          # 用统一临时密码
bash ~/gateway/add-email-user.sh alice@example.com 自定义密码  # 指定密码
bash ~/gateway/add-email-user.sh --del alice@example.com    # 删除
```

- 默认临时密码 `modifyme2026`,用户登录后到**设置 → 账户**自行修改。
- 密码经 bcrypt 存入 htpasswd,写完自动重启 oauth2。

## GitHub 白名单

在 `config.yaml` 的 `oauth.github_users` 里增删登录名,重新 `./deploy.sh gateway` 即可。

## 两种身份相互独立

同一个人用 GitHub 和用邮箱登录,是**两个独立身份**,各自的服务器凭据、工作区互不相通。
建议每人固定用一种。
