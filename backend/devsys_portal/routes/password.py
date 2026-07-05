"""POST /api/password — 邮箱登录用户自助改密。"""
from fastapi import APIRouter, Depends, HTTPException, Request

from ..auth import current_user
from ..htpasswd import is_email_user, restart_oauth2, set_password

router = APIRouter()


@router.post("/api/password")
async def change_password(request: Request, user: str = Depends(current_user)):
    if not is_email_user(user):
        raise HTTPException(403, "仅邮箱登录账号可改密")
    body = await request.json()
    pw = str(body.get("password", ""))
    if len(pw) < 8:
        raise HTTPException(400, "密码至少 8 位")
    try:
        set_password(user, pw)
        await restart_oauth2()
    except Exception as e:
        raise HTTPException(500, str(e)[:160])
    return {"ok": True}
