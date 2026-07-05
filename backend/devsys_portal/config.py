"""路径与环境配置。全部可用环境变量覆盖，便于部署时按机器定制。"""
import os
from pathlib import Path

BASE = Path(__file__).resolve().parent

# 运行期数据（用户凭据，Fernet 加密）——只存在于网关机
DATA = Path(os.environ.get("DEVSYS_DATA", "/var/lib/devsys-portal"))
# 内网服务器清单（由 deploy 从 config.yaml 渲染）
SERVERS_FILE = Path(os.environ.get("DEVSYS_SERVERS", "/etc/devsys/servers.json"))
# 文档目录（维护 *.md 即可）
DOCS = Path(os.environ.get("DEVSYS_DOCS", BASE.parent.parent / "docs"))
# 构建好的前端（Vite dist）。deploy 会把 frontend/dist 放到这里
WEB_DIR = Path(os.environ.get("DEVSYS_WEB", BASE.parent.parent / "frontend" / "dist"))
