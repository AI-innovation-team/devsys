"""POST /api/settings — 保存某服务器的用户名/认证方式，以及（可选）加密凭据。"""
from fastapi import APIRouter, Depends, HTTPException, Request
from fastapi.responses import JSONResponse

from ..auth import current_user
from ..servers import find_server
from ..storage import load_meta, save_meta, secret_path, write_secret

router = APIRouter()


@router.post("/api/settings")
async def settings(request: Request, user: str = Depends(current_user)):
    body = await request.json()
    if not find_server(body.get("server", "")):
        raise HTTPException(400, "unknown server")
    server = body["server"]
    meta = load_meta(user)
    entry = meta.get(server, {})
    entry["username"] = str(body.get("username", "")).strip()
    entry["auth"] = "key" if body.get("auth") == "key" else "password"
    meta[server] = entry
    save_meta(user, meta)
    if body.get("secret"):
        write_secret(user, server, body["secret"])
    return JSONResponse({"ok": True, "has_secret": secret_path(user, server).exists()})
