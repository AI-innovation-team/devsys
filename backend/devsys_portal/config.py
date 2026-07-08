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
# 邮箱登录用户表（oauth2-proxy htpasswd）——门户自助改密用
HTPASSWD = Path(os.environ.get("DEVSYS_HTPASSWD", BASE.parent.parent / "oauth2" / "htpasswd"))
# oauth2-proxy 配置文件——门户运行时**原地**改 github_users / htpasswd 行（绝不重生成整个 cfg，
# 否则丢 client_secret/cookie_secret 导致认证瘫痪、全员登录失效）。
OAUTH2_CFG = Path(os.environ.get("DEVSYS_OAUTH2_CFG", HTPASSWD.parent / "oauth2-proxy.cfg"))
# 改密后重启的 oauth2 服务名（需 zzl-zgh 免密 sudo restart，见 deploy/gateway/sudoers）
OAUTH2_SERVICE = os.environ.get("DEVSYS_OAUTH2_SERVICE", "devsys-oauth2")
# 管理员用户名集合（逗号分隔，由 deploy 从 config.yaml 的 oauth.admins 注入 DEVSYS_ADMINS）
ADMINS = set(filter(None, os.environ.get("DEVSYS_ADMINS", "").split(",")))
