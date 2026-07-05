"""内网服务器清单。由 deploy 从 config.yaml 渲染成 servers.json。

每项：{name, host, port, jump?}
  jump 可选：空=从门户所在机(bastion)直连；填另一台 server 的 name=经它 ProxyJump。
"""
import json

from .config import SERVERS_FILE


def servers() -> list:
    return json.loads(SERVERS_FILE.read_text()) if SERVERS_FILE.exists() else []


def find_server(name: str):
    return next((s for s in servers() if s["name"] == name), None)
