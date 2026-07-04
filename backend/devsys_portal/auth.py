"""身份来自 oauth2-proxy 注入的 X-Auth-Request-User（GitHub 用户名）。

Caddy 经 forward_auth 校验后把该头透传给门户；门户本身不做登录。
"""
from typing import Optional

from fastapi import Header, HTTPException


def check_user(u: Optional[str]) -> str:
    if not u:
        raise HTTPException(401, "未认证（缺少 X-Auth-Request-User）")
    if not all(c.isalnum() or c in "-_" for c in u):
        raise HTTPException(400, "bad user")
    return u


def current_user(x_auth_request_user: Optional[str] = Header(default=None)) -> str:
    """FastAPI 依赖：HTTP 路由用 Depends(current_user)。"""
    return check_user(x_auth_request_user)
