"""VS Code：点开 → SSH 起 code-server → 端口转发 → 按 /vscode/{server}/ 子路径代理（HTTP+WS）。

code-server 用相对路径，子路径代理天然可行。
"""
import asyncio

import httpx
import websockets as wsclient
from fastapi import (APIRouter, Depends, HTTPException, Request, WebSocket)
from fastapi.responses import HTMLResponse, RedirectResponse, StreamingResponse
from starlette.background import BackgroundTask

from ..auth import check_user, current_user
from ..servers import find_server
from ..vscode import drop_session, ensure_codeserver

router = APIRouter()

_http = httpx.AsyncClient(timeout=None, follow_redirects=False)
HOP = {"connection", "keep-alive", "proxy-authenticate", "proxy-authorization",
       "te", "trailers", "transfer-encoding", "upgrade"}


async def aclose():
    await _http.aclose()


@router.get("/vscode/{server}")
async def enter(server: str, user: str = Depends(current_user)):
    if not find_server(server):
        raise HTTPException(404, "unknown server")
    try:
        await ensure_codeserver(user, server)
    except Exception as e:
        return HTMLResponse(f"<pre style='color:#eee;background:#111;padding:20px'>"
                            f"启动 VS Code 失败：{e}</pre>", status_code=500)
    return RedirectResponse(url=f"/vscode/{server}/", status_code=302)


@router.websocket("/vscode/{server}/{path:path}")
async def ws_proxy(ws: WebSocket, server: str, path: str):
    try:
        user = check_user(ws.headers.get("x-auth-request-user"))
        port = await ensure_codeserver(user, server)
    except Exception:
        await ws.close()
        return
    subs = [p.strip() for p in ws.headers.get("sec-websocket-protocol", "").split(",") if p.strip()]
    await ws.accept(subprotocol=subs[0] if subs else None)
    target = f"ws://127.0.0.1:{port}/{path}"
    if ws.url.query:
        target += f"?{ws.url.query}"
    try:
        async with wsclient.connect(target, max_size=None, open_timeout=20,
                                    ping_interval=None, subprotocols=subs or None) as up:
            async def c2s():
                while True:
                    m = await ws.receive()
                    if m["type"] == "websocket.disconnect":
                        break
                    if m.get("text") is not None:
                        await up.send(m["text"])
                    elif m.get("bytes") is not None:
                        await up.send(m["bytes"])

            async def s2c():
                async for msg in up:
                    if isinstance(msg, bytes):
                        await ws.send_bytes(msg)
                    else:
                        await ws.send_text(msg)

            tasks = [asyncio.ensure_future(c2s()), asyncio.ensure_future(s2c())]
            await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            for t in tasks:
                t.cancel()
    except Exception:
        pass
    finally:
        try:
            await ws.close()
        except Exception:
            pass


@router.api_route("/vscode/{server}/{path:path}",
                  methods=["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"])
async def http_proxy(server: str, path: str, request: Request, user: str = Depends(current_user)):
    try:
        port = await ensure_codeserver(user, server)
    except Exception as e:
        raise HTTPException(502, str(e))
    url = f"http://127.0.0.1:{port}/{path}"
    headers = {k: v for k, v in request.headers.items()
               if k.lower() not in HOP and k.lower() != "host"}
    body = await request.body()
    req = _http.build_request(request.method, url, params=request.query_params,
                              headers=headers, content=body)
    try:
        resp = await _http.send(req, stream=True)
    except Exception:
        drop_session(user, server)   # 转发失败：会话可能已死，下次重建
        raise HTTPException(502, "code-server 连接中断，请刷新")
    rh = {k: v for k, v in resp.headers.items() if k.lower() not in HOP}
    return StreamingResponse(resp.aiter_raw(), status_code=resp.status_code,
                             headers=rh, background=BackgroundTask(resp.aclose))
