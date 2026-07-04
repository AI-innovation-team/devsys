"""devsys 门户后端：纯 JSON/WS API + 托管前端 SPA。

模块划分：
  config    路径/环境       crypto   Fernet 主密钥
  storage   per-user 凭据    servers  内网服务器清单
  auth      身份（oauth2-proxy 传入头）
  ssh       asyncssh 连接（含可选跳板）
  tmux      工作区（持久会话）    vscode  code-server 起停+转发
  docs      md 文档
  routes/*  FastAPI 路由
  main      app 组装
"""
__all__ = ["create_app"]
