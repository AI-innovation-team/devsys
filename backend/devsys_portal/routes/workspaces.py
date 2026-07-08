"""工作区（tmux 持久会话）：列举 / 新建 / 关闭。"""
import asyncio
import shlex

from fastapi import APIRouter, Depends, HTTPException, Request

from .. import audit
from ..auth import current_user
from ..servers import find_server, servers
from ..storage import load_meta, secret_path
from ..tmux import LIST_FMT, TMUX, WS_NAME, ok_name, parse_sessions, run

router = APIRouter()


@router.get("/api/workspaces")
async def list_ws(user: str = Depends(current_user)):
    meta = load_meta(user)

    async def one(s):
        name = s["name"]
        base = {"server": name, "host": s["host"], "port": s.get("port", 22),
                "jump": s.get("jump"), "sessions": [], "error": None}
        if not (meta.get(name, {}).get("username") and secret_path(user, name).exists()):
            return {**base, "configured": False}
        try:
            _, out, _ = await run(user, name, f"{TMUX} list-sessions -F '{LIST_FMT}' 2>/dev/null || true")
            return {**base, "configured": True, "sessions": parse_sessions(out)}
        except Exception as e:
            return {**base, "configured": True, "error": str(e)[:140]}

    return {"servers": await asyncio.gather(*[one(s) for s in servers()])}


@router.post("/api/workspaces/new")
async def new_ws(request: Request, user: str = Depends(current_user)):
    body = await request.json()
    server = body.get("server", "")
    name = str(body.get("name", "")).strip()
    if not find_server(server):
        raise HTTPException(400, "unknown server")
    if not WS_NAME.match(name):
        raise HTTPException(400, "工作区名仅限字母/数字/-/_/.，不以 - 开头，最长 64 位")
    try:
        await run(user, server, f"{TMUX} new-session -d -s {shlex.quote(name)} 2>&1 || true")
    except Exception as e:
        raise HTTPException(502, str(e)[:160])
    audit.record(user, "ws_new", server=server, ws=name)
    return {"ok": True}


@router.post("/api/workspaces/kill")
async def kill_ws(request: Request, user: str = Depends(current_user)):
    body = await request.json()
    server = body.get("server", "")
    name = str(body.get("name", "")).strip()
    if not find_server(server) or not ok_name(name):
        raise HTTPException(400, "bad request")
    try:
        await run(user, server, f"{TMUX} kill-session -t {shlex.quote(name)} 2>&1 || true")
    except Exception as e:
        raise HTTPException(502, str(e)[:160])
    return {"ok": True}
