"""GET /api/me — 当前用户 + 各服务器（含用户名/认证方式/是否已存凭据）。"""
from fastapi import APIRouter, Depends

from ..auth import current_user
from ..config import ADMINS
from ..htpasswd import is_email_user
from ..servers import servers
from ..storage import load_meta, secret_path

router = APIRouter()


@router.get("/api/me")
def me(user: str = Depends(current_user)):
    meta = load_meta(user)
    out = []
    for s in servers():
        m = meta.get(s["name"], {})
        out.append({**s,
                    "username": m.get("username", ""),
                    "auth": m.get("auth", "password"),
                    "has_secret": secret_path(user, s["name"]).exists()})
    return {"user": user, "email_login": is_email_user(user), "is_admin": user in ADMINS, "servers": out}
