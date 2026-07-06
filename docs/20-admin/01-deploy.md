# 一键部署

devsys 用一份 `config.yaml` 描述拓扑,一条命令渲染 + 构建 + 推送。

## 准备

```bash
cp .env.example .env         # 填 OAuth + DNS 凭据
cp config.yaml.example config.yaml
vim config.yaml              # 填域名、relay、网关、服务器
```

## 部署

```bash
./deploy.sh                  # 全量:渲染 + 构建前端 + 推 relay + 推 gateway
```

分步执行:

```bash
./deploy.sh render           # 只渲染网关配置
./deploy.sh build            # 只构建前端
./deploy.sh gateway          # 只推网关
./deploy.sh check            # 只检查配置,不连远程
```

## 结构

- **relay**:公网机,只做反向隧道转发。
- **gateway**:内网网关,跑 Caddy + oauth2-proxy + 门户后端。
- **servers**:目标机,门户以每个用户自己的账号 SSH 连接。
