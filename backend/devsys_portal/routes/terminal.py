"""Web SSH：/ws/ssh/{server}[?ws=<工作区>] —— asyncssh PTY ↔ WebSocket 桥接。

有 ws 参数 → 接入/新建持久 tmux 会话；无 → 临时 shell。
"""
import asyncio
import json

from fastapi import APIRouter, WebSocket, WebSocketDisconnect

from ..auth import check_user
from ..ssh import ssh_connect
from ..tmux import attach_cmd, ok_name

router = APIRouter()


@router.websocket("/ws/ssh/{server}")
async def ws_ssh(ws: WebSocket, server: str):
    await ws.accept()
    wsname = ws.query_params.get("ws") or None
    if wsname is not None and not ok_name(wsname):
        await ws.send_text("\r\n[devsys] 非法工作区名\r\n")
        await ws.close()
        return
    try:
        user = check_user(ws.headers.get("x-auth-request-user"))
        conn = await ssh_connect(user, server)
    except Exception as e:
        await ws.send_text(f"\r\n[devsys] {e}\r\n")
        await ws.close()
        return
    async with conn:
        cmd = attach_cmd(wsname) if wsname else None
        proc = await conn.create_process(cmd, term_type="xterm-256color", term_size=(80, 24))

        async def to_ssh():
            while True:
                data = await ws.receive_text()
                try:
                    obj = json.loads(data)
                except Exception:
                    proc.stdin.write(data)
                    continue
                if obj.get("t") == "r":
                    proc.change_terminal_size(int(obj["c"]), int(obj["r"]))
                elif obj.get("t") == "i":
                    proc.stdin.write(obj["d"])

        async def to_ws():
            while not proc.stdout.at_eof():
                out = await proc.stdout.read(4096)
                if not out:
                    break
                await ws.send_text(out)

        tasks = [asyncio.ensure_future(to_ssh()), asyncio.ensure_future(to_ws())]
        try:
            await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
        except WebSocketDisconnect:
            pass
        finally:
            for t in tasks:
                t.cancel()
            try:
                await ws.close()
            except Exception:
                pass
