"""身份来自 oauth2-proxy 注入的 X-Auth-Request-User（GitHub 用户名）。

Caddy 经 forward_auth 校验后把该头透传给门户；门户本身不做登录。
"""
from typing import Optional

from fastapi import Depends, Header, HTTPException

from .config import ADMINS


def check_user(u: Optional[str]) -> str:
    # 身份可能是 GitHub 用户名或邮箱（htpasswd 登录）。用作数据目录名，需挡路径穿越。
    if not u:
        raise HTTPException(401, "未认证（缺少 X-Auth-Request-User）")
    if ".." in u or u.startswith(".") or "/" in u or not all(c.isalnum() or c in "-_.@+" for c in u):
        raise HTTPException(400, "bad user")
    return u


def current_user(x_auth_request_user: Optional[str] = Header(default=None)) -> str:
    """FastAPI 依赖：HTTP 路由用 Depends(current_user)。"""
    return check_user(x_auth_request_user)


def require_admin(user: str = Depends(current_user)) -> str:
    """FastAPI 依赖：仅管理员（config.yaml oauth.admins）可访问的路由用 Depends(require_admin)。"""
    if user not in ADMINS:
        raise HTTPException(403, "需要管理员权限")
    return user
