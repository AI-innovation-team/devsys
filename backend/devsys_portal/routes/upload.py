"""高速上传：浏览器把「文件/目录」打成 tar.gz 流，分块可续传地传到门户，
finalize 时门户把整包**流式管进目标机的 `tar xzf`**（门户不常驻大文件、不占内存）。

为什么这么设计（见文档 功能详解 › 高速上传）：
  · 公网只有 relay 这一扇门，且是脆弱的 TCP 转发——分块 + 续传让上传抗断。
  · gzip 在浏览器端做（原生 CompressionStream），对代码/配置等于把带宽乘数倍。
  · 目录在浏览器端打成单条 tar 流，消灭成百上千个小文件的往返。
  · 落到 turing 的临时文件仅作续传缓冲，extract 后即删；解包在目标机本地完成。

协议（均需登录，且只有创建者本人能操作自己的上传）：
  GET    /api/upload/resolve?server=X&path=Y  -> {path,exists,isdir}（展开 ~/相对为绝对 + 查存在）
  POST   /api/upload/init    {server,dest,filename,total?} -> {id}
  PUT    /api/upload/{id}?offset=N   body=分块字节           -> {received}
  GET    /api/upload/{id}                                    -> {received,done}
  POST   /api/upload/{id}/finish                             -> {ok,detail?}
  DELETE /api/upload/{id}                                    -> {ok}
"""
import asyncio
import json
import re
import secrets
import shlex
import time
from pathlib import Path

from fastapi import APIRouter, Depends, HTTPException, Request

from ..auth import current_user
from ..config import DATA
from ..servers import find_server
from ..ssh import ssh_connect
from ..tmux import run

router = APIRouter()

UPLOADS = DATA / "uploads"
ID_RE = re.compile(r"^[0-9a-f]{24}$")
CHUNK_CAP = 16 * 1024 * 1024   # 单块上限（挡住异常巨块，正常客户端用 ~8MB）
_locks: dict[str, asyncio.Lock] = {}


def _paths(uid: str) -> tuple[Path, Path]:
    return UPLOADS / f"{uid}.data", UPLOADS / f"{uid}.json"


def _load_meta(uid: str, user: str) -> dict:
    """取上传记录并校验归属；不存在/非本人一律 404（不泄露存在性）。"""
    if not ID_RE.match(uid):
        raise HTTPException(404, "no such upload")
    _, meta_p = _paths(uid)
    if not meta_p.exists():
        raise HTTPException(404, "no such upload")
    meta = json.loads(meta_p.read_text())
    if meta.get("user") != user:
        raise HTTPException(404, "no such upload")
    return meta


def _cleanup(uid: str) -> None:
    for p in _paths(uid):
        try:
            p.unlink()
        except FileNotFoundError:
            pass
    _locks.pop(uid, None)


def _gc(ttl: int = 86400) -> None:
    """清理超期未完成的残留上传（如 finish 连接失败后被放弃的临时包）。init 时机会性调用。"""
    if not UPLOADS.exists():
        return
    now = time.time()
    for p in UPLOADS.glob("*.data"):
        try:
            if now - p.stat().st_mtime > ttl:
                _cleanup(p.stem)
        except OSError:
            pass


def _extract_cmd(dest: str) -> str:
    """构造目标机解包命令：mkdir -p 目标目录后流式解 tar.gz。
    dest 语义：空/~ = 家目录；~/x 或相对路径 = 相对家目录；/x = 绝对路径。
    用户本就有该机 shell 权限，这里不是权限边界，只做 tilde/相对的规范化。"""
    d = (dest or "").strip()
    if d in ("", "~"):
        base, rel = "cd ~ && ", "."
    elif d.startswith("~/"):
        base, rel = "cd ~ && ", d[2:]
    elif d.startswith("/"):
        base, rel = "", d
    else:
        base, rel = "cd ~ && ", d
    q = shlex.quote(rel)
    return f"{base}mkdir -p {q} && exec tar xzf - -C {q} --no-same-owner"


def _resolve_script(dest: str) -> str:
    """在目标机上把用户填的 dest 展开成绝对路径并查存在性。dest 语义同 _extract_cmd。
    realpath -m 可规范化不存在的路径（用于展示完整路径）。"""
    q = shlex.quote(dest or "")
    return (
        f'd={q}; '
        'case "$d" in '
        '""|"~") p="$HOME";; '
        '"~/"*) p="$HOME/${d#??}";; '
        '/*) p="$d";; '
        '*) p="$HOME/$d";; '
        'esac; '
        'abs=$(realpath -m -- "$p" 2>/dev/null || printf "%s" "$p"); '
        'printf "PATH=%s\\nEXISTS=%s\\nISDIR=%s\\n" "$abs" '
        '"$([ -e "$abs" ] && echo 1 || echo 0)" '
        '"$([ -d "$abs" ] && echo 1 || echo 0)"'
    )


@router.get("/api/upload/resolve")
async def resolve(server: str, path: str = "", user: str = Depends(current_user)):
    """把目标目录展开成绝对路径并核验是否存在（供前端实时显示完整路径 + 校验）。"""
    if not find_server(server):
        raise HTTPException(400, "unknown server")
    try:
        _, out, _ = await run(user, server, _resolve_script(path))
    except Exception as e:
        return {"path": "", "exists": False, "isdir": False, "error": str(e)[:140]}
    info: dict[str, str] = {}
    for line in out.splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            info[k] = v
    return {"path": info.get("PATH", ""), "exists": info.get("EXISTS") == "1", "isdir": info.get("ISDIR") == "1"}


@router.post("/api/upload/init")
async def init(request: Request, user: str = Depends(current_user)):
    body = await request.json()
    server = str(body.get("server", ""))
    dest = str(body.get("dest", "") or "")
    filename = str(body.get("filename", "") or "上传")
    if not find_server(server):
        raise HTTPException(400, "unknown server")
    UPLOADS.mkdir(parents=True, exist_ok=True)
    _gc()
    uid = secrets.token_hex(12)
    data_p, meta_p = _paths(uid)
    data_p.write_bytes(b"")
    meta_p.write_text(json.dumps({
        "user": user, "server": server, "dest": dest,
        "filename": filename[:200], "created": int(time.time()),
        "total": int(body.get("total") or 0),
    }))
    return {"id": uid}


@router.put("/api/upload/{uid}")
async def chunk(uid: str, request: Request, user: str = Depends(current_user)):
    _load_meta(uid, user)  # 校验归属
    try:
        offset = int(request.query_params.get("offset", "0"))
    except ValueError:
        raise HTTPException(400, "bad offset")
    data_p, _ = _paths(uid)
    lock = _locks.setdefault(uid, asyncio.Lock())
    async with lock:
        size = data_p.stat().st_size
        # 续传对齐：偏移必须紧接已收字节；不匹配则回报当前位置让客户端重对齐
        if offset != size:
            raise HTTPException(409, detail={"received": size})
        payload = await request.body()
        if len(payload) > CHUNK_CAP:
            raise HTTPException(413, "chunk too large")
        if payload:
            with data_p.open("ab") as f:
                f.write(payload)
        return {"received": data_p.stat().st_size}


@router.get("/api/upload/{uid}")
async def status(uid: str, user: str = Depends(current_user)):
    _load_meta(uid, user)
    data_p, _ = _paths(uid)
    return {"received": data_p.stat().st_size if data_p.exists() else 0, "done": False}


@router.post("/api/upload/{uid}/finish")
async def finish(uid: str, user: str = Depends(current_user)):
    meta = _load_meta(uid, user)
    data_p, _ = _paths(uid)
    if not data_p.exists() or data_p.stat().st_size == 0:
        _cleanup(uid)
        raise HTTPException(400, "空上传")
    cmd = _extract_cmd(meta["dest"])
    # 连接目标机失败通常是临时/可修复的（凭据、网络）——保留临时包让客户端重试 finish，
    # 不必重传（可能是 GB 级）。残留由 init 时的超期 GC 兜底清理。
    try:
        conn = await asyncio.wait_for(ssh_connect(user, meta["server"]), 20)
    except Exception as e:
        raise HTTPException(502, f"连接目标机失败：{str(e)[:160]}")
    try:
        async with conn:
            # encoding=None → 二进制模式：stdin 收 bytes、stderr 出 bytes
            proc = await conn.create_process(cmd, encoding=None)
            # 把临时包按块写入远程 tar 的 stdin（门户零缓冲、内网 SSH 快）。
            # tar x 几乎不产出 stdout，故只写 stdin、不必并发排空 stdout。
            loop = asyncio.get_event_loop()
            with data_p.open("rb") as f:
                while True:
                    block = await loop.run_in_executor(None, f.read, 1 << 20)
                    if not block:
                        break
                    proc.stdin.write(block)
                    await proc.stdin.drain()
            proc.stdin.write_eof()
            err = await asyncio.wait_for(proc.stderr.read(), 600)
            res = await asyncio.wait_for(proc.wait(), 30)
    except asyncio.TimeoutError:
        raise HTTPException(504, "目标机解包超时")
    except Exception as e:
        raise HTTPException(502, f"解包失败：{str(e)[:200]}")
    if res.exit_status != 0:
        emsg = err.decode("utf-8", "replace").strip() if isinstance(err, (bytes, bytearray)) else str(err)
        raise HTTPException(502, f"tar 解包失败：{emsg[:200] or res.exit_status}")
    _cleanup(uid)
    return {"ok": True}


@router.delete("/api/upload/{uid}")
async def cancel(uid: str, user: str = Depends(current_user)):
    _load_meta(uid, user)
    _cleanup(uid)
    return {"ok": True}
