"""devsys 门户：服务器列表 + 每人自设(userid+密钥/密码) + web SSH + web VS Code。
身份来自 oauth2-proxy 传入的 X-Auth-Request-User（GitHub 用户名）。
凭据用 Fernet 加密落盘。
web SSH：/ssh/{server} + /ws/ssh/{server}（asyncssh）。
web VS Code：点开 → SSH 进目标机以你的身份起 code-server → 端口转发 → 门户按 /vscode/{server}/ 子路径代理。"""
import asyncio
import json
import os
import re
import shlex
from pathlib import Path
from typing import Optional

import asyncssh
import httpx
import websockets as wsclient
from cryptography.fernet import Fernet
from fastapi import (FastAPI, Header, HTTPException, Request, WebSocket,
                     WebSocketDisconnect)
from fastapi.responses import (HTMLResponse, JSONResponse, RedirectResponse,
                               StreamingResponse)
from fastapi.staticfiles import StaticFiles
from starlette.background import BackgroundTask

DATA = Path(os.environ.get("DEVSYS_DATA", "/var/lib/devsys-portal"))
SERVERS_FILE = Path(os.environ.get("DEVSYS_SERVERS", "/etc/devsys/servers.json"))
STATIC = Path(__file__).parent / "static"
DOCS = Path(os.environ.get("DEVSYS_DOCS", Path(__file__).parent / "docs"))

app = FastAPI()
app.mount("/static", StaticFiles(directory=str(STATIC)), name="static")
_http = httpx.AsyncClient(timeout=None, follow_redirects=False)
HOP = {"connection", "keep-alive", "proxy-authenticate", "proxy-authorization",
       "te", "trailers", "transfer-encoding", "upgrade"}


# ── 存储 ───────────────────────────────────────────────────────────
def _fernet() -> Fernet:
    kf = DATA / "portal.key"
    if not kf.exists():
        DATA.mkdir(parents=True, exist_ok=True)
        kf.write_bytes(Fernet.generate_key())
        kf.chmod(0o600)
    return Fernet(kf.read_bytes())


def servers():
    return json.loads(SERVERS_FILE.read_text()) if SERVERS_FILE.exists() else []


def find_server(name: str):
    return next((s for s in servers() if s["name"] == name), None)


def current_user(u: Optional[str]) -> str:
    if not u:
        raise HTTPException(401, "未认证（缺少 X-Auth-Request-User）")
    if not all(c.isalnum() or c in "-_" for c in u):
        raise HTTPException(400, "bad user")
    return u


def user_dir(user: str) -> Path:
    d = DATA / "users" / user
    (d / "secrets").mkdir(parents=True, exist_ok=True)
    return d


def load_meta(user: str) -> dict:
    f = user_dir(user) / "meta.json"
    return json.loads(f.read_text()) if f.exists() else {}


def save_meta(user: str, meta: dict):
    (user_dir(user) / "meta.json").write_text(json.dumps(meta, indent=2))


def secret_path(user: str, server: str) -> Path:
    return user_dir(user) / "secrets" / f"{server}.enc"


def write_secret(user: str, server: str, plaintext: str):
    p = secret_path(user, server)
    p.write_bytes(_fernet().encrypt(plaintext.encode()))
    p.chmod(0o600)


def read_secret(user: str, server: str) -> Optional[str]:
    p = secret_path(user, server)
    return _fernet().decrypt(p.read_bytes()).decode() if p.exists() else None


# ── SSH 连接（供 web SSH 与 VS Code 复用）──────────────────────────
async def ssh_connect(user: str, server: str):
    srv = find_server(server)
    meta = load_meta(user).get(server, {})
    secret = read_secret(user, server)
    if not srv or not meta.get("username") or secret is None:
        raise RuntimeError("未配置该服务器的用户名/凭据，请先在门户设置")

    def kw_for(s, m, sec):
        kw = dict(host=s["host"], port=int(s.get("port", 22)),
                  username=m["username"], known_hosts=None)
        if m.get("auth") == "key":
            kw["client_keys"] = [asyncssh.import_private_key(sec)]
        else:
            kw["password"] = sec
        return kw

    tunnel = None
    jump = srv.get("jump")
    if jump:
        jsrv, jmeta, jsec = find_server(jump), load_meta(user).get(jump, {}), read_secret(user, jump)
        if not (jsrv and jmeta.get("username") and jsec):
            raise RuntimeError(f"跳板 {jump} 的用户名/凭据未配置")
        tunnel = await asyncssh.connect(**kw_for(jsrv, jmeta, jsec))
    return await asyncssh.connect(tunnel=tunnel, **kw_for(srv, meta, secret))


# ── API ────────────────────────────────────────────────────────────
@app.get("/api/me")
def api_me(x_auth_request_user: Optional[str] = Header(default=None)):
    user = current_user(x_auth_request_user)
    meta = load_meta(user)
    out = []
    for s in servers():
        m = meta.get(s["name"], {})
        out.append({**s, "username": m.get("username", ""),
                    "auth": m.get("auth", "password"),
                    "has_secret": secret_path(user, s["name"]).exists()})
    return {"user": user, "servers": out}


@app.post("/api/settings")
async def api_settings(request: Request,
                       x_auth_request_user: Optional[str] = Header(default=None)):
    user = current_user(x_auth_request_user)
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


# ── 文档（维护 docs/*.md 即可）─────────────────────────────────────
def _doc_files():
    return sorted([p for p in DOCS.glob("*.md") if p.is_file()], key=lambda p: p.name) if DOCS.exists() else []


def _doc_title(p: Path) -> str:
    try:
        for line in p.read_text(encoding="utf-8").splitlines():
            s = line.strip()
            if s.startswith("# "):
                return s[2:].strip()
    except Exception:
        pass
    return p.stem


@app.get("/api/docs")
def api_docs(x_auth_request_user: Optional[str] = Header(default=None)):
    current_user(x_auth_request_user)
    return {"docs": [{"slug": p.stem, "title": _doc_title(p)} for p in _doc_files()]}


@app.get("/api/docs/{slug}")
def api_doc(slug: str, x_auth_request_user: Optional[str] = Header(default=None)):
    current_user(x_auth_request_user)
    p = next((f for f in _doc_files() if f.stem == slug), None)  # 只匹配真实文件名，杜绝路径穿越
    if not p:
        raise HTTPException(404, "文档不存在")
    return {"slug": p.stem, "title": _doc_title(p), "content": p.read_text(encoding="utf-8")}


# ── 工作区（tmux 持久会话）─────────────────────────────────────────
# 用默认 socket，与用户手动 `tmux ls` 看到的是同一批会话（统一管理）。
WS_NAME = re.compile(r"^[A-Za-z0-9_.][A-Za-z0-9_.-]{0,63}$")  # 新建：友好名，不以 - 开头
TMUX = "tmux"


def _ok_name(n: str) -> bool:
    # 接入/关闭既有会话：名字来自 tmux 本身，可能较随意；只挡换行与超长，命令层用 shlex 转义。
    return bool(n) and "\n" not in n and "\r" not in n and len(n) <= 128
LIST_FMT = "#{session_name}|#{session_created}|#{session_windows}|#{session_attached}|#{session_activity}"


async def _run(user: str, server: str, cmd: str, timeout: float = 12.0):
    conn = await asyncio.wait_for(ssh_connect(user, server), timeout)
    try:
        r = await asyncio.wait_for(conn.run(cmd, check=False), timeout)
        return r.exit_status, r.stdout, r.stderr
    finally:
        conn.close()


def _parse_sessions(out: str):
    sess = []
    for line in out.splitlines():
        p = line.split("|")
        if len(p) < 4 or not p[0]:
            continue
        sess.append({"name": p[0],
                     "created": int(p[1]) if p[1].isdigit() else 0,
                     "windows": int(p[2]) if p[2].isdigit() else 1,
                     "attached": p[3] not in ("", "0"),
                     "activity": int(p[4]) if len(p) > 4 and p[4].isdigit() else 0})
    return sess


@app.get("/api/workspaces")
async def api_workspaces(x_auth_request_user: Optional[str] = Header(default=None)):
    user = current_user(x_auth_request_user)
    meta = load_meta(user)

    async def one(s):
        name = s["name"]
        base = {"server": name, "host": s["host"], "port": s.get("port", 22),
                "jump": s.get("jump"), "sessions": [], "error": None}
        if not (meta.get(name, {}).get("username") and secret_path(user, name).exists()):
            return {**base, "configured": False}
        try:
            _, out, _ = await _run(user, name, f"{TMUX} list-sessions -F '{LIST_FMT}' 2>/dev/null || true")
            return {**base, "configured": True, "sessions": _parse_sessions(out)}
        except Exception as e:
            return {**base, "configured": True, "error": str(e)[:140]}

    results = await asyncio.gather(*[one(s) for s in servers()])
    return {"servers": results}


@app.post("/api/workspaces/new")
async def api_ws_new(request: Request, x_auth_request_user: Optional[str] = Header(default=None)):
    user = current_user(x_auth_request_user)
    body = await request.json()
    server = body.get("server", "")
    name = str(body.get("name", "")).strip()
    if not find_server(server):
        raise HTTPException(400, "unknown server")
    if not WS_NAME.match(name):
        raise HTTPException(400, "工作区名仅限字母/数字/-/_/.，不以 - 开头，最长 64 位")
    try:
        await _run(user, server, f"{TMUX} new-session -d -s {shlex.quote(name)} 2>&1 || true")
    except Exception as e:
        raise HTTPException(502, str(e)[:160])
    return {"ok": True}


@app.post("/api/workspaces/kill")
async def api_ws_kill(request: Request, x_auth_request_user: Optional[str] = Header(default=None)):
    user = current_user(x_auth_request_user)
    body = await request.json()
    server = body.get("server", "")
    name = str(body.get("name", "")).strip()
    if not find_server(server) or not _ok_name(name):
        raise HTTPException(400, "bad request")
    try:
        await _run(user, server, f"{TMUX} kill-session -t {shlex.quote(name)} 2>&1 || true")
    except Exception as e:
        raise HTTPException(502, str(e)[:160])
    return {"ok": True}


# ── web SSH ────────────────────────────────────────────────────────
@app.websocket("/ws/ssh/{server}")
async def ws_ssh(ws: WebSocket, server: str):
    await ws.accept()
    wsname = ws.query_params.get("ws") or None
    if wsname is not None and not _ok_name(wsname):
        await ws.send_text("\r\n[devsys] 非法工作区名\r\n")
        await ws.close()
        return
    try:
        user = current_user(ws.headers.get("x-auth-request-user"))
        conn = await ssh_connect(user, server)
    except Exception as e:
        await ws.send_text(f"\r\n[devsys] {e}\r\n")
        await ws.close()
        return
    async with conn:
        # 有 ws → 接入/新建持久 tmux 会话（-A 有则接入、-D 踢掉旧连接）；无 → 临时 shell
        cmd = f"{TMUX} new-session -A -D -s {shlex.quote(wsname)}" if wsname else None
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


@app.get("/ssh/{server}", response_class=HTMLResponse)
def ssh_page(server: str, x_auth_request_user: Optional[str] = Header(default=None)):
    current_user(x_auth_request_user)
    if not find_server(server):
        raise HTTPException(404, "unknown server")
    return TERM_HTML.replace("__SERVER__", server)


# ── web VS Code（per-user code-server over SSH）────────────────────
_cs_sessions = {}   # (user, server) -> {"conn":..., "listener":..., "port":local}
_cs_locks = {}

# home 是跨机共享 NFS，端口文件与 user-data-dir 必须按主机名隔离，否则互相打架。
LAUNCHER = r"""
set -e
export PATH="$HOME/.local/bin:$PATH"
H=$(hostname -s)
mkdir -p ~/.devsys
PF=~/.devsys/cs.$H.port
if [ -f "$PF" ]; then P=$(cat "$PF"); curl -sf "http://127.0.0.1:$P/healthz" >/dev/null 2>&1 && { echo "$P"; exit 0; }; fi
command -v code-server >/dev/null || { echo NO_CODESERVER; exit 0; }
P=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
setsid code-server --bind-addr 127.0.0.1:$P --auth none --disable-telemetry \
  --disable-update-check --user-data-dir ~/.devsys/cs-data-$H >~/.devsys/cs.$H.log 2>&1 &
echo "$P" > "$PF"
for i in $(seq 1 40); do curl -sf "http://127.0.0.1:$P/healthz" >/dev/null 2>&1 && break; sleep 0.5; done
echo "$P"
"""


async def ensure_codeserver(user: str, server: str) -> int:
    key = (user, server)
    sess = _cs_sessions.get(key)
    if sess:
        return sess["port"]
    lock = _cs_locks.setdefault(key, asyncio.Lock())
    async with lock:
        if key in _cs_sessions:
            return _cs_sessions[key]["port"]
        conn = await ssh_connect(user, server)
        r = await conn.run(LAUNCHER, check=False)
        last = (r.stdout.strip().splitlines() or [""])[-1]
        if last == "NO_CODESERVER":
            conn.close()
            raise RuntimeError(f"目标机 {server} 未安装 code-server")
        if not last.isdigit():
            conn.close()
            raise RuntimeError(f"code-server 启动失败: {r.stdout.strip()[-200:]}")
        listener = await conn.forward_local_port("127.0.0.1", 0, "127.0.0.1", int(last))
        _cs_sessions[key] = {"conn": conn, "listener": listener, "port": listener.get_port()}
        return _cs_sessions[key]["port"]


def _drop_session(user: str, server: str):
    sess = _cs_sessions.pop((user, server), None)
    if sess:
        try:
            sess["listener"].close()
        except Exception:
            pass
        try:
            sess["conn"].close()
        except Exception:
            pass


@app.get("/vscode/{server}")
async def vscode_enter(server: str,
                       x_auth_request_user: Optional[str] = Header(default=None)):
    user = current_user(x_auth_request_user)
    if not find_server(server):
        raise HTTPException(404, "unknown server")
    try:
        await ensure_codeserver(user, server)
    except Exception as e:
        return HTMLResponse(f"<pre style='color:#eee;background:#111;padding:20px'>"
                            f"启动 VS Code 失败：{e}</pre>", status_code=500)
    return RedirectResponse(url=f"/vscode/{server}/", status_code=302)


@app.websocket("/vscode/{server}/{path:path}")
async def vscode_ws(ws: WebSocket, server: str, path: str):
    try:
        user = current_user(ws.headers.get("x-auth-request-user"))
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
                                    ping_interval=None,
                                    subprotocols=subs or None) as up:
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


@app.api_route("/vscode/{server}/{path:path}",
               methods=["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"])
async def vscode_http(server: str, path: str, request: Request,
                      x_auth_request_user: Optional[str] = Header(default=None)):
    user = current_user(x_auth_request_user)
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
        _drop_session(user, server)   # 转发失败：可能会话已死，下次重建
        raise HTTPException(502, "code-server 连接中断，请刷新")
    rh = {k: v for k, v in resp.headers.items() if k.lower() not in HOP}
    return StreamingResponse(resp.aiter_raw(), status_code=resp.status_code,
                             headers=rh, background=BackgroundTask(resp.aclose))


# ── 首页 ───────────────────────────────────────────────────────────
@app.get("/", response_class=HTMLResponse)
def index(x_auth_request_user: Optional[str] = Header(default=None)):
    user = current_user(x_auth_request_user)
    return HOME_HTML.replace("__USER__", user)


CIBOL_TOKENS = """
@import url('https://fonts.googleapis.com/css2?family=Noto+Sans+SC:wght@400;500;700&family=Source+Sans+3:wght@400;500;600;700&family=Source+Serif+4:opsz,wght@8..60,400;8..60,500;8..60,600&family=JetBrains+Mono:wght@400;500;600&display=swap');
  :root{
    --font-sans:"Source Sans 3","Noto Sans SC",-apple-system,BlinkMacSystemFont,"Segoe UI",system-ui,sans-serif;
    --font-serif:"Source Serif 4","Noto Sans SC",Georgia,serif;
    --font-mono:"JetBrains Mono",ui-monospace,"SF Mono",Menlo,Consolas,monospace;
    --stone-50:#F6F3ED;--stone-100:#ECE7DD;--stone-200:#DBD4C6;--stone-300:#C3BAAA;--stone-400:#A39A8B;
    --stone-500:#837B6F;--stone-600:#66605A;--stone-700:#4E4A44;--stone-800:#38352F;--stone-900:#2A2A28;--stone-950:#1B1A18;
    --cream:#F5F3EE;--paper:#FBFAF6;
    --terracotta-50:#FBF0EA;--terracotta-200:#E9BCA3;--terracotta-300:#DC9B77;--terracotta-400:#CE7B50;
    --terracotta-500:#BD5D3A;--terracotta-600:#A54E2F;--terracotta-700:#883E26;
    --sage-100:#DEE7DD;--sage-300:#9DB8A2;--sage-500:#5C8A6B;--sage-600:#4C7458;
    --amber-100:#F6E7C4;--amber-400:#D6A23E;--amber-500:#BE8A28;
    --clay-100:#F4D7CF;--clay-400:#C8634A;--clay-500:#B23A2E;
    --slate-100:#D9E1E6;--slate-400:#6E92A6;--slate-500:#4E7596;
    --canvas:var(--paper);--surface:var(--paper);--surface-raised:#FFFFFF;--surface-sunken:var(--stone-50);--surface-hover:var(--stone-100);
    --text-strong:var(--stone-900);--text-body:var(--stone-800);--text-muted:var(--stone-600);--text-faint:var(--stone-500);--text-on-accent:var(--cream);
    --border-subtle:var(--stone-200);--border-default:var(--stone-300);--border-strong:var(--stone-400);
    --accent:var(--terracotta-500);--accent-hover:var(--terracotta-600);--accent-press:var(--terracotta-700);
    --accent-soft:var(--terracotta-50);--accent-soft-bd:var(--terracotta-200);--accent-text:var(--terracotta-600);
    --success:var(--sage-500);--success-soft:var(--sage-100);--success-text:var(--sage-600);
    --warning:var(--amber-500);--warning-soft:var(--amber-100);--warning-text:var(--amber-500);
    --danger:var(--clay-500);--danger-soft:var(--clay-100);--danger-text:var(--clay-500);
    --info:var(--slate-500);--info-soft:var(--slate-100);--info-text:var(--slate-500);
    --radius-sm:5px;--radius-md:8px;--radius-lg:12px;--radius-xl:16px;--radius-pill:999px;
    --shadow-xs:0 1px 2px rgba(42,42,40,.06);
    --shadow-sm:0 1px 2px rgba(42,42,40,.06),0 2px 6px rgba(42,42,40,.05);
    --shadow-md:0 2px 4px rgba(42,42,40,.06),0 6px 16px rgba(42,42,40,.08);
    --shadow-lg:0 4px 8px rgba(42,42,40,.07),0 16px 36px rgba(42,42,40,.12);
    --ring:0 0 0 3px var(--accent-soft);
    --ease-out:cubic-bezier(.22,.61,.36,1);--ease-spring:cubic-bezier(.34,1.46,.58,1);
    --dur-fast:120ms;--dur-base:200ms;--dur-slow:320ms;
    color-scheme:light;
  }
  [data-theme=dark]{
    --canvas:#232220;--surface:#232220;--surface-raised:#2C2A26;--surface-sunken:#2A2825;--surface-hover:#322F2A;
    --text-strong:#F2EEE4;--text-body:#DED8CB;--text-muted:#A39A8B;--text-faint:#837B6F;--text-on-accent:#1B1A18;
    --border-subtle:#353230;--border-default:#46423C;--border-strong:#5A554E;
    --accent:var(--terracotta-400);--accent-hover:var(--terracotta-300);--accent-press:var(--terracotta-200);
    --accent-soft:rgba(189,93,58,.16);--accent-soft-bd:rgba(206,123,80,.34);--accent-text:var(--terracotta-300);
    --success:var(--sage-300);--success-soft:rgba(92,138,107,.18);--success-text:var(--sage-300);
    --warning:var(--amber-400);--warning-soft:rgba(190,138,40,.18);--warning-text:var(--amber-400);
    --danger:var(--clay-400);--danger-soft:rgba(178,58,46,.2);--danger-text:var(--clay-400);
    --shadow-xs:0 1px 2px rgba(0,0,0,.4);
    --shadow-sm:0 1px 2px rgba(0,0,0,.4),0 2px 6px rgba(0,0,0,.32);
    --shadow-md:0 2px 4px rgba(0,0,0,.42),0 6px 16px rgba(0,0,0,.4);
    --shadow-lg:0 4px 8px rgba(0,0,0,.45),0 16px 36px rgba(0,0,0,.5);
    color-scheme:dark;
  }
  *,*::before,*::after{box-sizing:border-box}
  body{margin:0;font-family:var(--font-sans);font-size:15px;line-height:1.5;color:var(--text-body);
    background:var(--canvas);-webkit-font-smoothing:antialiased;text-rendering:optimizeLegibility;
    font-feature-settings:"kern","liga","calt"}
  h1,h2,h3{font-family:var(--font-sans);color:var(--text-strong);font-weight:600;line-height:1.3;letter-spacing:-.01em;margin:0;text-wrap:balance}
  a{color:var(--accent-text);text-decoration:none;transition:color var(--dur-fast) var(--ease-out)}
  a:hover{color:var(--accent-hover)}
  ::selection{background:var(--accent-soft);color:var(--accent-text)}
  ::-webkit-scrollbar{width:11px;height:11px}
  ::-webkit-scrollbar-thumb{background:var(--border-default);border-radius:999px;border:3px solid transparent;background-clip:content-box}
  ::-webkit-scrollbar-thumb:hover{background:var(--border-strong);background-clip:content-box}
  .eyebrow{font-size:11px;font-weight:600;letter-spacing:.14em;text-transform:uppercase;color:var(--text-faint)}
  .ic{display:inline-flex;flex-shrink:0}
  .ic svg{width:100%;height:100%;display:block}
"""


HOME_HTML = """<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>devsys · 内网门户</title>
<script>(function(){var t;try{t=localStorage.getItem('devsys.theme')}catch(e){}document.documentElement.setAttribute('data-theme',t==='dark'?'dark':'light');})();</script>
<script src="/static/marked.min.js"></script>
<style>__TOKENS__
  .app{display:flex;height:100vh;overflow:hidden;background:var(--canvas)}
  /* sidebar rail */
  .rail{width:240px;flex-shrink:0;height:100%;display:flex;flex-direction:column;
    background:var(--surface);border-right:1px solid var(--border-subtle);transition:width var(--dur-base) var(--ease-out)}
  .rail.collapsed{width:64px}
  .rail-top{display:flex;align-items:center;gap:8px;padding:14px 14px 4px}
  .rail.collapsed .rail-top{justify-content:center;padding:14px 0 4px}
  .rail-toggle{width:36px;height:36px;flex-shrink:0;display:inline-flex;align-items:center;justify-content:center;
    border:none;background:transparent;color:var(--text-muted);border-radius:8px;cursor:pointer;padding:0}
  .rail-toggle:hover{background:var(--surface-hover);color:var(--text-body)}
  .rail-toggle .ic{width:19px;height:19px}
  .brand{display:flex;align-items:center;gap:10px;min-width:0}
  .rail.collapsed .brand{display:none}
  .brand-tile{width:32px;height:32px;border-radius:8px;flex-shrink:0;display:inline-flex;align-items:center;justify-content:center;
    background:var(--stone-900);color:var(--cream)}
  .brand-tile .ic{width:19px;height:19px}
  [data-theme=dark] .brand-tile{background:var(--cream);color:var(--stone-900)}
  .brand-name{font-family:var(--font-serif);font-size:15px;font-weight:600;color:var(--text-strong);letter-spacing:.04em;line-height:1.2}
  .brand-sub{font-size:11px;color:var(--text-faint);letter-spacing:.06em;white-space:nowrap}
  .nav{flex:1;overflow-y:auto;padding:8px 12px 0;display:flex;flex-direction:column;gap:2px}
  .rail.collapsed .nav{padding:8px 10px 0}
  .nav-sec{font-size:11px;font-weight:600;letter-spacing:.14em;text-transform:uppercase;color:var(--text-faint);padding:10px 12px 6px}
  .rail.collapsed .nav-sec{display:none}
  .nav-item{position:relative;display:flex;align-items:center;gap:11px;width:100%;padding:9px 12px;border:none;text-align:left;
    background:transparent;color:var(--text-body);border-radius:8px;cursor:pointer;font-family:inherit;font-size:14px;font-weight:500;
    transition:background var(--dur-fast) var(--ease-out),color var(--dur-fast) var(--ease-out)}
  .rail.collapsed .nav-item{justify-content:center;gap:0;padding:10px 0}
  .nav-item:hover{background:var(--surface-hover)}
  .nav-item.active{background:var(--accent-soft);color:var(--accent-text);font-weight:600}
  .nav-item.active .ic{color:var(--accent)}
  .nav-item.active::before{content:"";position:absolute;left:-12px;top:50%;transform:translateY(-50%);width:3px;height:18px;background:var(--accent);border-radius:0 3px 3px 0}
  .rail.collapsed .nav-item.active::before{display:none}
  .nav-item.disabled{opacity:.55;cursor:default}
  .nav-item.disabled:hover{background:transparent}
  .nav-item .ic{width:20px;height:20px;color:var(--text-muted)}
  .nav-item .lbl{flex:1}
  .rail.collapsed .nav-item .lbl,.rail.collapsed .soon{display:none}
  .soon{font-size:10px;font-weight:600;color:var(--text-faint);background:var(--surface-sunken);border:1px solid var(--border-subtle);padding:1px 7px;border-radius:999px}
  .rail-bottom{padding:6px 12px}
  .rail.collapsed .rail-bottom{padding:6px 10px}
  .rail-foot{padding:8px 12px 14px;border-top:1px solid var(--border-subtle);margin-top:6px;position:relative}
  .rail.collapsed .rail-foot{padding:8px 8px 14px;display:flex;justify-content:center}
  .avatar-btn{display:flex;align-items:center;gap:10px;width:100%;padding:5px 6px;border:none;background:transparent;cursor:pointer;border-radius:8px}
  .avatar-btn:hover{background:var(--surface-hover)}
  .rail.collapsed .avatar-btn{justify-content:center;padding:0;width:auto}
  .avatar{width:32px;height:32px;border-radius:50%;flex-shrink:0;display:inline-flex;align-items:center;justify-content:center;
    background:var(--accent-soft);color:var(--accent-text);font-weight:600;font-size:13px;border:1px solid var(--accent-soft-bd);text-transform:uppercase}
  .avatar-meta{min-width:0;text-align:left;flex:1}
  .avatar-name{font-weight:600;font-size:13px;color:var(--text-strong);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .avatar-sub{font-size:11px;color:var(--text-faint)}
  .avatar-chev{width:16px;height:16px;color:var(--text-faint)}
  .rail.collapsed .avatar-meta,.rail.collapsed .avatar-chev{display:none}
  .menu{position:absolute;bottom:calc(100% + 6px);left:12px;right:auto;min-width:200px;background:var(--surface-raised);
    border:1px solid var(--border-subtle);border-radius:8px;box-shadow:var(--shadow-lg);padding:6px;z-index:60;display:none}
  .rail.collapsed .menu{left:8px}
  .menu.open{display:block}
  .menu a,.menu button{display:flex;align-items:center;gap:10px;width:100%;padding:9px 12px;border:none;background:transparent;
    cursor:pointer;text-align:left;font-family:inherit;font-size:14px;color:var(--text-body);border-radius:5px;text-decoration:none}
  .menu a:hover,.menu button:hover{background:var(--surface-hover)}
  .menu .ic{width:18px;height:18px;color:var(--text-muted)}
  .menu .danger,.menu .danger .ic{color:var(--danger-text)}
  .menu-sep{height:1px;background:var(--border-subtle);margin:5px 6px}
  /* main content */
  .main{flex:1;overflow-y:auto;min-width:0}
  .wrap{max-width:900px;margin:0 auto;padding:40px 34px 64px}
  .page-head{margin-bottom:28px}
  .page-head h1{font-size:32px;margin-top:8px;letter-spacing:-.02em}
  .page-head>p{color:var(--text-muted);margin-top:8px;font-size:14.5px;max-width:580px;line-height:1.65}
  .stats{display:flex;gap:28px;margin-top:22px}
  .stat .v{font-family:var(--font-serif);font-size:27px;font-weight:600;color:var(--text-strong);font-variant-numeric:tabular-nums;line-height:1}
  .stat .l{font-size:11px;letter-spacing:.12em;text-transform:uppercase;color:var(--text-faint);margin-top:5px}
  /* server card */
  .cards{display:flex;flex-direction:column;gap:16px}
  .card{background:var(--surface);border:1px solid var(--border-subtle);border-radius:12px;box-shadow:var(--shadow-sm);
    overflow:hidden;transition:border-color var(--dur-base) var(--ease-out),box-shadow var(--dur-base) var(--ease-out)}
  .card:hover{border-color:var(--border-default);box-shadow:var(--shadow-md)}
  .card-head{display:flex;justify-content:space-between;align-items:flex-start;gap:16px;padding:20px 22px}
  .srv-title{display:flex;align-items:center;gap:9px;flex-wrap:wrap}
  .srv-dot{width:8px;height:8px;border-radius:50%;background:var(--text-faint);flex-shrink:0}
  .srv-dot.ok{background:var(--success)}
  .srv-name{font-size:19px;font-weight:600;color:var(--text-strong)}
  .srv-host{font-family:var(--font-mono);font-size:12.5px;color:var(--text-muted);margin-top:7px;display:flex;align-items:center;gap:6px}
  .srv-host .ic{width:14px;height:14px;color:var(--text-faint)}
  .srv-actions{display:flex;gap:8px;flex-shrink:0}
  .badge{display:inline-flex;align-items:center;gap:5px;height:22px;padding:0 9px;border-radius:999px;font-size:12px;font-weight:600;
    letter-spacing:-.01em;background:var(--surface-sunken);color:var(--text-muted);border:1px solid var(--border-subtle)}
  .badge .ic{width:13px;height:13px}
  .badge.info{background:var(--info-soft);color:var(--info-text);border-color:transparent}
  .badge.ok{background:var(--success-soft);color:var(--success-text);border-color:transparent}
  .badge.warn{background:var(--warning-soft);color:var(--warning-text);border-color:transparent}
  /* buttons */
  .btn{display:inline-flex;align-items:center;justify-content:center;gap:7px;height:38px;padding:0 16px;font-family:var(--font-sans);
    font-size:14px;font-weight:600;letter-spacing:-.01em;line-height:1;border:1px solid transparent;border-radius:8px;cursor:pointer;
    white-space:nowrap;user-select:none;text-decoration:none;box-shadow:var(--shadow-xs);
    transition:background var(--dur-fast) var(--ease-out),transform var(--dur-fast) var(--ease-out),box-shadow var(--dur-fast) var(--ease-out)}
  .btn .ic{width:17px;height:17px}
  .btn:active:not(:disabled){transform:translateY(.5px)}
  .btn.primary{background:var(--accent);color:var(--text-on-accent)}
  .btn.primary:hover:not(:disabled){background:var(--accent-hover)}
  .btn.secondary{background:var(--surface);color:var(--text-strong);border-color:var(--border-default)}
  .btn.secondary:hover:not(:disabled){background:var(--surface-hover)}
  .btn.sm{height:32px;padding:0 13px;font-size:13px}
  .btn:disabled{opacity:.45;cursor:not-allowed;box-shadow:none}
  /* credential panel */
  .cred{border-top:1px solid var(--border-subtle);padding:16px 22px}
  .cred-toggle{display:flex;align-items:center;justify-content:space-between;width:100%;background:none;border:none;padding:0;
    cursor:pointer;font-family:inherit;color:var(--text-muted)}
  .cred-hint{display:inline-flex;align-items:center;gap:7px;font-size:13px;font-weight:500;color:var(--text-muted)}
  .chev{width:15px;height:15px;transition:transform var(--dur-base) var(--ease-out)}
  .cred-body.open ~ .cred-toggle .chev{transform:rotate(180deg)}
  .cred-body{display:none;margin-top:16px}
  .cred-body.open{display:block}
  .row2{display:grid;grid-template-columns:1fr 1fr;gap:12px}
  .field{margin-bottom:12px}
  .field:last-child{margin-bottom:0}
  .field label{display:block;margin-bottom:6px;font-size:13px;font-weight:500;color:var(--text-body)}
  .inp{display:flex;align-items:center;gap:8px;height:38px;padding:0 12px;background:var(--surface);
    border:1px solid var(--border-default);border-radius:8px;transition:border-color var(--dur-fast),box-shadow var(--dur-fast)}
  .inp:focus-within{border-color:var(--accent);box-shadow:var(--ring)}
  .inp .ic{width:17px;height:17px;color:var(--text-faint)}
  .inp input,.inp select{flex:1;min-width:0;height:100%;border:none;outline:none;background:transparent;color:var(--text-strong);font-family:inherit;font-size:14px}
  .inp select{cursor:pointer}
  .ta{width:100%;min-height:94px;padding:11px 12px;background:var(--surface);border:1px solid var(--border-default);border-radius:8px;
    color:var(--text-strong);font-family:var(--font-mono);font-size:12.5px;line-height:1.55;resize:vertical;outline:none;
    transition:border-color var(--dur-fast),box-shadow var(--dur-fast)}
  .ta:focus{border-color:var(--accent);box-shadow:var(--ring)}
  .pw-field{}
  .key-field{display:none}
  .card.auth-key .pw-field{display:none}
  .card.auth-key .key-field{display:block}
  .cred-foot{display:flex;align-items:center;gap:14px;margin-top:14px}
  .save-note{font-size:13px;color:var(--success-text);min-height:18px}
  /* views + settings */
  .view[hidden]{display:none}
  .set-sec{margin-top:30px}
  .set-h{margin-bottom:14px}
  .set-h h2{font-size:18px;font-weight:600;color:var(--text-strong);letter-spacing:-.01em}
  .set-h p{font-size:13.5px;color:var(--text-muted);margin-top:4px}
  .cfg-head{display:flex;justify-content:space-between;align-items:center;gap:12px;padding:15px 22px;border-bottom:1px solid var(--border-subtle);flex-wrap:wrap}
  .cfg-body{padding:18px 22px}
  .set-row{display:flex;align-items:center;gap:16px;justify-content:space-between;flex-wrap:wrap}
  .set-row .lbl2{font-size:14px;font-weight:500;color:var(--text-body)}
  .set-row .lbl2 small{display:block;font-size:12.5px;color:var(--text-faint);font-weight:400;margin-top:2px}
  .seg{display:inline-flex;background:var(--surface-sunken);border:1px solid var(--border-subtle);border-radius:8px;padding:3px;gap:2px}
  .seg button{height:32px;padding:0 20px;border:none;background:transparent;color:var(--text-muted);font-family:inherit;font-size:13px;font-weight:600;border-radius:6px;cursor:pointer;transition:background var(--dur-fast) var(--ease-out),color var(--dur-fast) var(--ease-out)}
  .seg button.on{background:var(--surface-raised);color:var(--text-strong);box-shadow:var(--shadow-xs)}
  .btn.subtle{background:var(--surface-sunken);color:var(--text-body);box-shadow:none}
  .btn.subtle:hover:not(:disabled){background:var(--surface-hover)}
  .btn.danger{background:var(--danger);color:#fff;border-color:transparent}
  .btn.danger:hover:not(:disabled){filter:brightness(1.05)}
  /* workspaces */
  .ph-row{display:flex;justify-content:space-between;align-items:flex-start;gap:12px}
  .ws-group{margin-bottom:24px}
  .ws-ghead{display:flex;justify-content:space-between;align-items:center;gap:14px;margin-bottom:12px;flex-wrap:wrap}
  .ws-new{display:flex;gap:8px;align-items:center}
  .ws-new input{height:32px;width:190px;max-width:52vw;border:1px solid var(--border-default);border-radius:8px;background:var(--surface);
    padding:0 12px;font-family:var(--font-mono);font-size:13px;color:var(--text-strong);outline:none;transition:border-color var(--dur-fast),box-shadow var(--dur-fast)}
  .ws-new input:focus{border-color:var(--accent);box-shadow:var(--ring)}
  .ws-list{display:flex;flex-direction:column;gap:10px}
  .ws-row{display:flex;justify-content:space-between;align-items:center;gap:14px;background:var(--surface);
    border:1px solid var(--border-subtle);border-radius:10px;padding:12px 16px;box-shadow:var(--shadow-xs);flex-wrap:wrap;
    transition:border-color var(--dur-base),box-shadow var(--dur-base)}
  .ws-row:hover{border-color:var(--border-default);box-shadow:var(--shadow-sm)}
  .ws-info{display:flex;align-items:center;gap:10px;min-width:0;flex-wrap:wrap}
  .ws-dot{width:8px;height:8px;border-radius:50%;background:var(--text-faint);flex-shrink:0}
  .ws-dot.on{background:var(--success)}
  .ws-name{font-family:var(--font-mono);font-size:14px;font-weight:600;color:var(--text-strong)}
  .ws-time{font-size:12px;color:var(--text-faint);display:inline-flex;align-items:center;gap:5px}
  .ws-time .ic{width:12px;height:12px}
  .ws-acts{display:flex;gap:8px;flex-shrink:0}
  .ws-acts .btn .ic{width:16px;height:16px}
  .ws-empty{padding:15px 2px;color:var(--text-muted);font-size:13.5px;display:flex;align-items:center;gap:7px}
  .ws-empty .ic{width:15px;height:15px}
  .ws-empty.err{color:var(--danger-text)}
  .ws-empty a{cursor:pointer;color:var(--accent-text)}
  .ws-new-bar{display:flex;align-items:center;gap:14px;flex-wrap:wrap;background:var(--surface);
    border:1px solid var(--border-subtle);border-radius:12px;padding:14px 18px;margin-bottom:22px;box-shadow:var(--shadow-xs)}
  .nb-label{font-size:13.5px;font-weight:600;color:var(--text-strong)}
  .inp.sel{height:34px;width:auto;min-width:132px;flex:0 0 auto}
  .inp.sel select{cursor:pointer}
  .badge.accent{background:var(--accent-soft);color:var(--accent-text);border-color:transparent}
  .badge.accent .ic{width:12px;height:12px}
  .ws-note{margin-top:16px;font-size:12.5px;color:var(--text-faint)}
  .ws-note a{cursor:pointer;color:var(--accent-text)}
  code{font-family:var(--font-mono);font-size:.9em;background:var(--surface-sunken);border:1px solid var(--border-subtle);border-radius:4px;padding:1px 5px}
  /* docs */
  .docs-wrap{display:flex;max-width:1040px;margin:0 auto;align-items:flex-start}
  .docs-side{width:216px;flex-shrink:0;padding:38px 16px 40px 28px;position:sticky;top:0;align-self:flex-start}
  .docs-list{display:flex;flex-direction:column;gap:2px;margin-top:12px}
  .docs-item{display:block;width:100%;text-align:left;padding:8px 12px;border:none;background:transparent;color:var(--text-body);
    border-radius:8px;cursor:pointer;font-family:inherit;font-size:14px;font-weight:500;transition:background var(--dur-fast) var(--ease-out)}
  .docs-item:hover{background:var(--surface-hover)}
  .docs-item.active{background:var(--accent-soft);color:var(--accent-text);font-weight:600}
  .docs-main{flex:1;min-width:0;padding:38px 32px 72px;border-left:1px solid var(--border-subtle)}
  .md-body{max-width:720px;font-size:15.5px;line-height:1.75;color:var(--text-body)}
  .md-body>*:first-child{margin-top:0}
  .md-body h1{font-size:30px;margin:4px 0 10px;letter-spacing:-.02em;color:var(--text-strong)}
  .md-body h2{font-size:22px;margin:34px 0 12px;padding-bottom:8px;border-bottom:1px solid var(--border-subtle);color:var(--text-strong)}
  .md-body h3{font-size:18px;margin:26px 0 8px;color:var(--text-strong)}
  .md-body p{margin:14px 0}
  .md-body a{color:var(--accent-text);text-decoration:underline;text-underline-offset:2px}
  .md-body ul,.md-body ol{margin:14px 0;padding-left:24px}
  .md-body li{margin:6px 0}
  .md-body li>ul,.md-body li>ol{margin:6px 0}
  .md-body code{font-family:var(--font-mono);font-size:.88em;background:var(--surface-sunken);border:1px solid var(--border-subtle);border-radius:5px;padding:1.5px 6px}
  .md-body pre{background:var(--surface-sunken);border:1px solid var(--border-subtle);border-radius:10px;padding:14px 16px;overflow-x:auto;margin:16px 0}
  .md-body pre code{background:none;border:none;padding:0;font-size:13px;line-height:1.65}
  .md-body blockquote{margin:16px 0;padding:2px 16px;border-left:3px solid var(--accent);color:var(--text-muted)}
  .md-body blockquote p{margin:8px 0}
  .md-body table{border-collapse:collapse;margin:18px 0;font-size:14px;display:block;overflow-x:auto}
  .md-body th,.md-body td{border:1px solid var(--border-subtle);padding:8px 13px;text-align:left}
  .md-body th{background:var(--surface-sunken);font-weight:600;color:var(--text-strong)}
  .md-body img{max-width:100%;border-radius:8px}
  .md-body hr{border:none;border-top:1px solid var(--border-subtle);margin:28px 0}
  @media (max-width:760px){
    .app{flex-direction:column;height:auto;min-height:100vh;overflow:visible}
    .rail{width:100%!important;height:auto;flex-direction:row;align-items:center;border-right:none;
      border-bottom:1px solid var(--border-subtle);padding:10px 16px;gap:12px}
    .rail .nav,.rail .nav-sec,.rail .rail-bottom,.rail-toggle{display:none}
    .rail-top{padding:0;flex:1}
    .rail-foot{padding:0;border-top:none;margin-top:0;width:auto}
    .brand{display:flex!important}
    .menu{left:auto;right:0;bottom:auto;top:calc(100% + 6px);min-width:180px}
    .main{overflow:visible}
    .wrap{padding:24px 16px 48px}
    .card-head,.cred,.cfg-head,.cfg-body{padding-left:16px;padding-right:16px}
    .card-head{flex-direction:column}
    .srv-actions{width:100%}.srv-actions .btn{flex:1}
    .row2{grid-template-columns:1fr}
    .stats{gap:22px}
    .docs-wrap{flex-direction:column}
    .docs-side{width:100%;position:static;padding:20px 16px 0}
    .docs-list{flex-direction:row;overflow-x:auto;gap:6px;margin-top:10px}
    .docs-item{white-space:nowrap;width:auto}
    .docs-main{border-left:none;padding:22px 16px 56px}
  }
</style></head>
<body>
<div class="app">
  <nav class="rail" id="rail">
    <div class="rail-top">
      <button class="rail-toggle" onclick="toggleRail()" title="收起 / 展开" aria-label="收起或展开侧栏"><span class="ic" id="railIc"></span></button>
      <div class="brand">
        <span class="brand-tile"><span class="ic" id="brandIc"></span></span>
        <div><div class="brand-name">devsys</div><div class="brand-sub">内网开发者门户</div></div>
      </div>
    </div>
    <div class="nav">
      <div class="nav-sec">工作台</div>
      <button class="nav-item active" data-view="workspaces" onclick="go('workspaces')"><span class="ic" data-ic="grid"></span><span class="lbl">工作区</span></button>
      <button class="nav-item" data-view="servers" onclick="go('servers')"><span class="ic" data-ic="terminal"></span><span class="lbl">服务器</span></button>
      <button class="nav-item" data-view="docs" onclick="go('docs')"><span class="ic" data-ic="file"></span><span class="lbl">文档</span></button>
    </div>
    <div class="rail-bottom">
      <button class="nav-item" onclick="toggleTheme()"><span class="ic" id="themeIc"></span><span class="lbl" id="themeLbl">深色模式</span></button>
    </div>
    <div class="rail-foot">
      <button class="avatar-btn" onclick="toggleMenu(event)" aria-label="用户菜单">
        <span class="avatar" id="avatar">·</span>
        <div class="avatar-meta"><div class="avatar-name">__USER__</div><div class="avatar-sub">GitHub 身份</div></div>
        <span class="ic avatar-chev" data-ic="updown"></span>
      </button>
      <div class="menu" id="menu">
        <button onclick="go('settings')"><span class="ic" data-ic="settings"></span>设置</button>
        <div class="menu-sep"></div>
        <a href="/oauth2/sign_out" class="danger"><span class="ic" data-ic="logout"></span>退出登录</a>
      </div>
    </div>
  </nav>
  <main class="main">
    <div class="wrap view" id="view-workspaces">
      <header class="page-head">
        <div class="ph-row">
          <div>
            <div class="eyebrow">workspaces</div>
            <h1>工作区</h1>
          </div>
          <button class="btn subtle sm" onclick="loadWorkspaces()"><span class="ic" data-ic="refresh"></span>刷新</button>
        </div>
        <p>你的持久会话常驻在服务器上（基于 tmux，与你在机器上 <code>tmux ls</code> 看到的是同一批）。关掉网页只是断开，回来点「打开」即可继续接入 —— 进程与终端历史原样还在。</p>
      </header>
      <div class="ws-new-bar" id="wsNewBar"></div>
      <div id="wsGroups"></div>
    </div>
    <div class="wrap view" id="view-servers" hidden>
      <header class="page-head">
        <div class="eyebrow">developer gateway</div>
        <h1>服务器</h1>
        <p>选择目标机，从浏览器打开 Web SSH 或 VS Code。连接始终以你自己的身份进行。</p>
        <div class="stats">
          <div class="stat"><div class="v" id="mTotal">0</div><div class="l">Servers</div></div>
          <div class="stat"><div class="v" id="mReady">0</div><div class="l">Ready</div></div>
          <div class="stat"><div class="v" id="mUnset">0</div><div class="l">Unset</div></div>
        </div>
      </header>
      <div class="cards" id="cards"></div>
    </div>
    <div class="wrap view" id="view-settings" hidden>
      <header class="page-head">
        <div class="eyebrow">settings</div>
        <h1>设置</h1>
        <p>管理你连接各内网服务器的凭据与偏好。凭据经 Fernet 加密后仅存于门户，连接时以你的身份进行。</p>
      </header>
      <section class="set-sec">
        <div class="set-h"><h2>连接凭据</h2><p>为每台服务器设置你在该机上的 userid 与密钥 / 密码。</p></div>
        <div class="cards" id="credCards"></div>
      </section>
      <section class="set-sec">
        <div class="set-h"><h2>外观</h2><p>切换门户的浅色 / 深色主题。</p></div>
        <div class="card"><div class="cfg-body"><div class="set-row">
          <span class="lbl2">主题<small>选择你偏好的界面配色</small></span>
          <div class="seg" id="themeSeg">
            <button data-t="light" onclick="setTheme('light')">浅色</button>
            <button data-t="dark" onclick="setTheme('dark')">深色</button>
          </div>
        </div></div></div>
      </section>
    </div>
    <div class="view" id="view-docs" hidden>
      <div class="docs-wrap">
        <aside class="docs-side">
          <div class="eyebrow">docs</div>
          <div class="docs-list" id="docsList"></div>
        </aside>
        <article class="docs-main">
          <div class="md-body" id="docBody"><div class="ws-empty">加载中…</div></div>
        </article>
      </div>
    </div>
  </main>
</div>
<script>
// ── lucide 图标（细描边）──
const P={
  terminal:'<path d="M4 17l6-6-6-6"/><path d="M12 19h8"/>',
  code:'<path d="m16 18 6-6-6-6"/><path d="m8 6-6 6 6 6"/>',
  network:'<rect x="16" y="16" width="6" height="6" rx="1"/><rect x="2" y="16" width="6" height="6" rx="1"/><rect x="9" y="2" width="6" height="6" rx="1"/><path d="M5 16v-3a1 1 0 0 1 1-1h12a1 1 0 0 1 1 1v3"/><path d="M12 12V8"/>',
  user:'<circle cx="12" cy="8" r="5"/><path d="M20 21a8 8 0 0 0-16 0"/>',
  key:'<path d="m15.5 7.5 2.3 2.3a1 1 0 0 0 1.4 0l2.1-2.1a1 1 0 0 0 0-1.4L21 5"/><path d="m21 2-9.6 9.6"/><circle cx="7.5" cy="15.5" r="5.5"/>',
  lock:'<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>',
  save:'<path d="M15.2 3a2 2 0 0 1 1.4.6l3.8 3.8a2 2 0 0 1 .6 1.4V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"/><path d="M17 21v-7a1 1 0 0 0-1-1H8a1 1 0 0 0-1 1v7"/><path d="M7 3v4a1 1 0 0 0 1 1h7"/>',
  check:'<circle cx="12" cy="12" r="10"/><path d="m9 12 2 2 4-4"/>',
  alert:'<circle cx="12" cy="12" r="10"/><line x1="12" x2="12" y1="8" y2="12"/><line x1="12" x2="12.01" y1="16" y2="16"/>',
  chevron:'<path d="m6 9 6 6 6-6"/>',
  updown:'<path d="m7 15 5 5 5-5"/><path d="m7 9 5-5 5 5"/>',
  file:'<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M16 13H8"/><path d="M16 17H8"/><path d="M10 9H8"/>',
  settings:'<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>',
  logout:'<path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><polyline points="16 17 21 12 16 7"/><line x1="21" x2="9" y1="12" y2="12"/>',
  sun:'<circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/>',
  moon:'<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>',
  panel:'<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/>',
  grid:'<rect width="7" height="7" x="3" y="3" rx="1"/><rect width="7" height="7" x="14" y="3" rx="1"/><rect width="7" height="7" x="14" y="14" rx="1"/><rect width="7" height="7" x="3" y="14" rx="1"/>',
  plus:'<path d="M5 12h14"/><path d="M12 5v14"/>',
  trash:'<path d="M3 6h18"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><line x1="10" x2="10" y1="11" y2="17"/><line x1="14" x2="14" y1="11" y2="17"/>',
  refresh:'<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>',
  clock:'<circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/>',
  server:'<rect width="20" height="8" x="2" y="2" rx="2"/><rect width="20" height="8" x="2" y="14" rx="2"/><line x1="6" x2="6.01" y1="6" y2="6"/><line x1="6" x2="6.01" y1="18" y2="18"/>'
};
function svg(n){return '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">'+(P[n]||'')+'</svg>';}
function paintIcons(root){(root||document).querySelectorAll('.ic[data-ic]').forEach(e=>{if(!e.dataset.done){e.innerHTML=svg(e.dataset.ic);e.dataset.done='1';}});}
document.getElementById('railIc').innerHTML=svg('panel');
document.getElementById('brandIc').innerHTML=svg('terminal');

// ── 主题 ──
function isDark(){return document.documentElement.getAttribute('data-theme')==='dark';}
function syncTheme(){
  document.getElementById('themeIc').innerHTML=svg(isDark()?'sun':'moon');
  document.getElementById('themeLbl').textContent=isDark()?'浅色模式':'深色模式';
  document.querySelectorAll('#themeSeg button').forEach(b=>b.classList.toggle('on',b.dataset.t===(isDark()?'dark':'light')));
}
function setTheme(t){document.documentElement.setAttribute('data-theme',t);try{localStorage.setItem('devsys.theme',t)}catch(e){}syncTheme();}
function toggleTheme(){setTheme(isDark()?'light':'dark');}

// ── 侧栏折叠 ──
function toggleRail(){const r=document.getElementById('rail');const c=r.classList.toggle('collapsed');try{localStorage.setItem('devsys.rail',c?'1':'0')}catch(e){}}
try{if(localStorage.getItem('devsys.rail')==='1')document.getElementById('rail').classList.add('collapsed');}catch(e){}

// ── 头像菜单 ──
function toggleMenu(e){e.stopPropagation();document.getElementById('menu').classList.toggle('open');}
document.addEventListener('click',()=>document.getElementById('menu').classList.remove('open'));

// ── 视图切换 ──
function go(view){
  document.querySelectorAll('.view').forEach(v=>{v.hidden=(v.id!=='view-'+view);});
  document.querySelectorAll('.nav-item[data-view]').forEach(n=>n.classList.toggle('active',n.dataset.view===view));
  try{localStorage.setItem('devsys.view',view)}catch(e){}
  const m=document.querySelector('.main');if(m)m.scrollTop=0;
  document.getElementById('menu').classList.remove('open');
  if(view==='workspaces')loadWorkspaces();
  if(view==='docs')loadDocs();
}

// ── 服务器启动卡（服务器页）──
function launchCard(s){
  const ready=s.has_secret&&s.username;
  const act=(label,ic,url,cls)=>ready
    ?`<a class="btn ${cls}" href="${url}" target="_blank"><span class="ic">${svg(ic)}</span>${label}</a>`
    :`<button class="btn ${cls}" disabled title="请先在设置中配置凭据"><span class="ic">${svg(ic)}</span>${label}</button>`;
  const tag=s.jump?`<span class="badge">via ${s.jump}</span>`:`<span class="badge info">内网</span>`;
  const foot=ready?'':`<div class="cred"><button class="cred-toggle" onclick="go('settings')">
    <span class="badge warn"><span class="ic">${svg('alert')}</span>未设置凭据</span>
    <span class="cred-hint">前往设置<span class="ic chev" style="transform:rotate(-90deg)">${svg('chevron')}</span></span></button></div>`;
  return `<article class="card">
    <div class="card-head">
      <div>
        <div class="srv-title"><span class="srv-dot ${ready?'ok':''}"></span><span class="srv-name">${s.name}</span>${tag}</div>
        <div class="srv-host"><span class="ic">${svg('network')}</span>${s.host}:${s.port}</div>
      </div>
      <div class="srv-actions">
        ${act('SSH','terminal','/ssh/'+s.name,'secondary')}
        ${act('VS Code','code','/vscode/'+s.name,'primary')}
      </div>
    </div>${foot}
  </article>`;
}

// ── 凭据配置卡（设置页）──
function configCard(s){
  const keyOn=s.auth==='key';
  const ready=s.has_secret&&s.username;
  return `<article class="card ${keyOn?'auth-key':''}" data-srv="${s.name}">
    <div class="cfg-head">
      <div class="srv-title"><span class="srv-dot ${ready?'ok':''}"></span><span class="srv-name">${s.name}</span>
        <span class="badge">${s.host}:${s.port}</span>${s.jump?`<span class="badge">via ${s.jump}</span>`:''}</div>
      <span class="badge ${ready?'ok':'warn'}"><span class="ic">${svg(ready?'check':'alert')}</span>${ready?'凭据就绪':'未设置'}</span>
    </div>
    <div class="cfg-body">
      <div class="row2">
        <div class="field"><label>用户名 (userid)</label>
          <div class="inp"><span class="ic">${svg('user')}</span><input class="u" value="${s.username||''}" placeholder="如 zzl-zgh" autocomplete="off"></div></div>
        <div class="field"><label>认证方式</label>
          <div class="inp"><span class="ic">${svg('key')}</span><select class="a">
            <option value="password" ${keyOn?'':'selected'}>密码</option>
            <option value="key" ${keyOn?'selected':''}>SSH 私钥</option></select></div></div>
      </div>
      <div class="field pw-field"><label>密码</label>
        <div class="inp"><span class="ic">${svg('lock')}</span><input class="pw" type="password" placeholder="${s.has_secret?'已保存，留空则不改':'输入密码'}"></div></div>
      <div class="field key-field"><label>SSH 私钥</label>
        <textarea class="ta key" placeholder="${s.has_secret?'已保存，留空则不改':'-----BEGIN OPENSSH PRIVATE KEY-----'}"></textarea></div>
      <div class="cred-foot">
        <button class="btn primary sm save" onclick="save('${s.name}',this)"><span class="ic">${svg('save')}</span>保存凭据</button>
        <span class="save-note"></span>
      </div>
    </div></article>`;
}
function bind(el){el.querySelector('.a').addEventListener('change',e=>{el.classList.toggle('auth-key',e.target.value==='key');});}
async function load(){
  const me=await (await fetch('/api/me')).json();
  const ready=me.servers.filter(s=>s.has_secret&&s.username).length;
  mTotal.textContent=me.servers.length;mReady.textContent=ready;mUnset.textContent=me.servers.length-ready;
  document.getElementById('cards').innerHTML=me.servers.map(launchCard).join('');
  const cc=document.getElementById('credCards');
  cc.innerHTML=me.servers.map(configCard).join('');
  cc.querySelectorAll('.card').forEach(bind);
  document.getElementById('avatar').textContent=(me.user||'·').slice(0,1);
}
async function save(name,btn){
  const el=btn.closest('.card');const note=el.querySelector('.save-note');
  const auth=el.querySelector('.a').value;
  const secret=auth==='key'?el.querySelector('.key').value:el.querySelector('.pw').value;
  const body={server:name,username:el.querySelector('.u').value,auth};
  if(secret) body.secret=secret;
  btn.disabled=true;note.textContent='保存中…';
  try{
    await fetch('/api/settings',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)});
    note.textContent='已保存 ✓';setTimeout(load,650);
  }catch(e){note.textContent='保存失败';btn.disabled=false;}
}
// ── 工作区（tmux 持久会话）──
function ago(ts){if(!ts)return '';var s=Math.floor(Date.now()/1000)-ts;if(s<60)return '刚刚';if(s<3600)return Math.floor(s/60)+' 分钟前';if(s<86400)return Math.floor(s/3600)+' 小时前';return Math.floor(s/86400)+' 天前';}
function wsRow(sv,ss){
  return `<div class="ws-row">
    <div class="ws-info">
      <span class="ws-dot ${ss.attached?'on':''}"></span>
      <span class="ws-name">${ss.name}</span>
      <span class="badge accent"><span class="ic">${svg('server')}</span>${sv}</span>
      <span class="badge">${ss.windows} 窗口</span>
      ${ss.attached?`<span class="badge ok"><span class="ic">${svg('check')}</span>使用中</span>`:''}
      ${ss.activity?`<span class="ws-time"><span class="ic">${svg('clock')}</span>活跃 ${ago(ss.activity)}</span>`:''}
    </div>
    <div class="ws-acts">
      <a class="btn secondary sm" href="/ssh/${encodeURIComponent(sv)}?ws=${encodeURIComponent(ss.name)}" target="_blank"><span class="ic">${svg('terminal')}</span>打开</a>
      <button class="btn subtle sm" title="关闭工作区" data-sv="${sv}" data-nm="${encodeURIComponent(ss.name)}" onclick="killWs(this)"><span class="ic">${svg('trash')}</span></button>
    </div>
  </div>`;
}
function renderWorkspaces(d){
  const servers=d.servers||[];
  const cfg=servers.filter(s=>s.configured&&!s.error);
  const nb=document.getElementById('wsNewBar');
  if(cfg.length){
    nb.innerHTML=`<span class="nb-label">新建工作区</span>
      <div class="ws-new">
        <div class="inp sel"><span class="ic">${svg('server')}</span><select id="wsNewSrv">${cfg.map(s=>`<option value="${s.server}">${s.server}</option>`).join('')}</select></div>
        <input id="wsNewName" placeholder="工作区名（字母/数字）" maxlength="64" onkeydown="if(event.key==='Enter')newWs2()">
        <button class="btn primary sm" onclick="newWs2()"><span class="ic">${svg('plus')}</span>新建</button>
      </div>`;
  }else{
    nb.innerHTML=`<span class="nb-label" style="color:var(--text-muted)"><span class="ic" style="width:15px;height:15px;vertical-align:-2px;color:var(--warning)">${svg('alert')}</span> 还没有可用服务器 · <a onclick="go('settings')">前往设置配置凭据</a></span>`;
  }
  const rows=[];
  servers.forEach(s=>(s.sessions||[]).forEach(ss=>rows.push({sv:s.server,ss})));
  rows.sort((a,b)=>(b.ss.attached-a.ss.attached)||(b.ss.activity-a.ss.activity));
  const errs=servers.filter(s=>s.error).map(s=>`<div class="ws-empty err"><span class="ic">${svg('alert')}</span>${s.server} 无法连接：${s.error}</div>`).join('');
  let html=errs;
  if(rows.length) html+=`<div class="ws-list">${rows.map(r=>wsRow(r.sv,r.ss)).join('')}</div>`;
  else if(!errs) html+=`<div class="ws-empty">还没有工作区 · 在上方选服务器新建一个</div>`;
  const un=servers.filter(s=>!s.configured).length;
  if(un) html+=`<div class="ws-note">${un} 台服务器未配置凭据，不在列表内 · <a onclick="go('settings')">前往设置</a></div>`;
  document.getElementById('wsGroups').innerHTML=html;
}
async function loadWorkspaces(){
  document.getElementById('wsGroups').innerHTML='<div class="ws-empty">加载中…</div>';
  try{
    const d=await (await fetch('/api/workspaces')).json();
    renderWorkspaces(d);
  }catch(e){document.getElementById('wsGroups').innerHTML='<div class="ws-empty err">加载失败</div>';}
}
async function newWs2(){
  const sel=document.getElementById('wsNewSrv'),inp=document.getElementById('wsNewName');
  if(!sel||!inp)return;
  const sv=sel.value,name=(inp.value||'').trim();
  if(!/^[A-Za-z0-9_.][A-Za-z0-9_.-]{0,63}$/.test(name)){inp.focus();inp.style.borderColor='var(--danger)';return;}
  const btn=inp.nextElementSibling;btn.disabled=true;
  try{
    const r=await fetch('/api/workspaces/new',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({server:sv,name})});
    if(!r.ok)throw 0;
    window.open('/ssh/'+encodeURIComponent(sv)+'?ws='+encodeURIComponent(name),'_blank');
    inp.value='';loadWorkspaces();
  }catch(e){btn.disabled=false;}
}
function killWs(btn){
  if(btn.dataset.armed){clearTimeout(+btn.dataset.t);doKill(btn);return;}
  btn.dataset.armed='1';btn.classList.remove('subtle');btn.classList.add('danger');btn.textContent='确认关闭';
  btn.dataset.t=setTimeout(()=>{if(btn.dataset.armed){delete btn.dataset.armed;btn.classList.remove('danger');btn.classList.add('subtle');btn.innerHTML='<span class="ic">'+svg('trash')+'</span>';}},2600);
}
async function doKill(btn){
  delete btn.dataset.armed;btn.disabled=true;btn.textContent='关闭中…';
  const sv=btn.dataset.sv,name=decodeURIComponent(btn.dataset.nm);
  try{await fetch('/api/workspaces/kill',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({server:sv,name})});}catch(e){}
  loadWorkspaces();
}
// ── 文档 ──
function esc(s){return String(s).replace(/[&<>]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;'}[c]));}
let DOCS_LOADED=false;
async function loadDocs(force){
  if(DOCS_LOADED&&!force)return;
  const list=document.getElementById('docsList');
  try{
    const d=await (await fetch('/api/docs')).json();
    const docs=d.docs||[];
    if(!docs.length){list.innerHTML='';document.getElementById('docBody').innerHTML='<div class="ws-empty">还没有文档 · 在服务器 <code>~/gateway/portal/docs/</code> 放置 .md 文件即可</div>';DOCS_LOADED=true;return;}
    list.innerHTML=docs.map(x=>`<button class="docs-item" data-slug="${esc(x.slug)}" onclick="openDoc(this.dataset.slug,this)">${esc(x.title)}</button>`).join('');
    DOCS_LOADED=true;
    let last;try{last=localStorage.getItem('devsys.doc')}catch(e){}
    const pick=docs.some(x=>x.slug===last)?last:docs[0].slug;
    openDoc(pick);
  }catch(e){document.getElementById('docBody').innerHTML='<div class="ws-empty err">文档加载失败</div>';}
}
async function openDoc(slug){
  document.querySelectorAll('.docs-item').forEach(i=>i.classList.toggle('active',i.dataset.slug===slug));
  try{localStorage.setItem('devsys.doc',slug)}catch(e){}
  const body=document.getElementById('docBody');
  try{
    const d=await (await fetch('/api/docs/'+encodeURIComponent(slug))).json();
    body.innerHTML=window.marked?marked.parse(d.content):('<pre>'+esc(d.content)+'</pre>');
    const m=document.querySelector('.main');if(m)m.scrollTop=0;
  }catch(e){body.innerHTML='<div class="ws-empty err">无法打开文档</div>';}
}
if(window.marked&&marked.setOptions)marked.setOptions({gfm:true,breaks:false});
let iv;try{iv=localStorage.getItem('devsys.view')}catch(e){}
paintIcons();syncTheme();load();go(['servers','settings','workspaces','docs'].includes(iv)?iv:'workspaces');
</script>
</body></html>"""


TERM_HTML = """<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>SSH · __SERVER__</title>
<script>(function(){var t;try{t=localStorage.getItem('devsys.theme')}catch(e){}document.documentElement.setAttribute('data-theme',t==='dark'?'dark':'light');})();</script>
<link rel="stylesheet" href="/static/xterm.css">
<style>__TOKENS__
  html,body{height:100%}
  body{display:flex;flex-direction:column;background:var(--canvas)}
  .tbar{display:flex;justify-content:space-between;align-items:center;gap:16px;padding:11px 18px;
    background:var(--surface);border-bottom:1px solid var(--border-subtle)}
  .tbar .l{display:flex;align-items:center;gap:11px;min-width:0}
  .back{display:inline-flex;align-items:center;justify-content:center;width:34px;height:34px;border-radius:8px;
    color:var(--text-muted);border:1px solid var(--border-subtle);background:var(--surface)}
  .back:hover{background:var(--surface-hover);color:var(--text-body)}
  .back .ic{width:17px;height:17px}
  .tile{width:28px;height:28px;border-radius:7px;flex-shrink:0;display:inline-flex;align-items:center;justify-content:center;background:var(--stone-900);color:var(--cream)}
  [data-theme=dark] .tile{background:var(--cream);color:var(--stone-900)}
  .tile .ic{width:17px;height:17px}
  .tbrand{font-family:var(--font-serif);font-weight:600;font-size:15px;color:var(--text-strong);letter-spacing:.03em}
  .sp{display:inline-flex;align-items:center;gap:7px;height:26px;padding:0 11px;border-radius:999px;
    font-family:var(--font-mono);font-size:12px;color:var(--text-muted);background:var(--surface-sunken);border:1px solid var(--border-subtle)}
  .dot{width:7px;height:7px;border-radius:50%;background:var(--text-faint);transition:background .3s}
  .dot.on{background:var(--success)} .dot.off{background:var(--danger)}
  .stage{flex:1;padding:16px;min-height:0}
  .term{height:100%;display:flex;flex-direction:column;background:#1a1b1e;border:1px solid rgba(0,0,0,.55);
    border-radius:var(--radius-lg);box-shadow:0 4px 28px rgba(0,0,0,.4);overflow:hidden}
  .term.fs{border-radius:0;border:none}
  .term-title{display:flex;align-items:center;justify-content:space-between;height:40px;padding:0 14px;
    background:rgba(0,0,0,.28);border-bottom:1px solid rgba(255,255,255,.06)}
  .term-who{font-family:var(--font-mono);font-size:12.5px;color:rgba(230,230,230,.5);
    overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .fsbtn{flex-shrink:0;margin-left:10px;display:inline-flex;align-items:center;justify-content:center;width:28px;height:28px;
    border:none;background:transparent;border-radius:5px;cursor:pointer;color:rgba(230,230,230,.4);transition:color .15s,background .15s}
  .fsbtn:hover{color:rgba(230,230,230,.85);background:rgba(255,255,255,.09)}
  .fsbtn .ic{width:15px;height:15px}
  .term-body{flex:1;padding:10px 12px;min-height:0}
  #t{height:100%;width:100%}
</style></head>
<body>
<div class="tbar">
  <div class="l">
    <a class="back" href="/" title="返回门户"><span class="ic" id="backIc"></span></a>
    <span class="tile"><span class="ic" id="tileIc"></span></span>
    <span class="tbrand">devsys</span>
  </div>
  <span class="sp"><span class="dot" id="dot"></span><span id="who">__SERVER__</span></span>
</div>
<div class="stage"><div class="term" id="term">
  <div class="term-title">
    <span class="term-who" id="titleWho">__SERVER__</span>
    <button class="fsbtn" id="fsbtn" title="全屏"><span class="ic" id="fsIc"></span></button>
  </div>
  <div class="term-body"><div id="t"></div></div>
</div></div>
<script src="/static/xterm.js"></script>
<script src="/static/xterm-addon-fit.js"></script>
<script>
const P={arrowLeft:'<path d="m12 19-7-7 7-7"/><path d="M19 12H5"/>',terminal:'<path d="M4 17l6-6-6-6"/><path d="M12 19h8"/>',
  max:'<polyline points="15 3 21 3 21 9"/><polyline points="9 21 3 21 3 15"/><line x1="21" x2="14" y1="3" y2="10"/><line x1="3" x2="10" y1="21" y2="14"/>',
  min:'<polyline points="4 14 10 14 10 20"/><polyline points="20 10 14 10 14 4"/><line x1="14" x2="21" y1="10" y2="3"/><line x1="3" x2="10" y1="21" y2="14"/>'};
function svg(n){return '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">'+P[n]+'</svg>';}
document.getElementById('backIc').innerHTML=svg('arrowLeft');
document.getElementById('tileIc').innerHTML=svg('terminal');
document.getElementById('fsIc').innerHTML=svg('max');

const server="__SERVER__";
const wsName=new URLSearchParams(location.search).get('ws')||'';
const dot=document.getElementById('dot');
const term=new Terminal({fontSize:13.5,scrollback:5000,
  fontFamily:'"JetBrains Mono",ui-monospace,SFMono-Regular,Menlo,monospace',cursorBlink:true,
  theme:{background:'#1a1b1e',foreground:'#e6e6e6',cursor:'#7FB069',selectionBackground:'rgba(127,176,105,.28)'}});
const fit=new FitAddon.FitAddon();term.loadAddon(fit);
term.open(document.getElementById('t'));fit.fit();term.focus();

// 全屏
const box=document.getElementById('term');
document.getElementById('fsbtn').onclick=()=>{document.fullscreenElement?document.exitFullscreen():box.requestFullscreen&&box.requestFullscreen();};
document.addEventListener('fullscreenchange',()=>{const on=!!document.fullscreenElement;box.classList.toggle('fs',on);
  document.getElementById('fsIc').innerHTML=svg(on?'min':'max');setTimeout(()=>{sendResize();term.focus();},80);});

// 标题栏 user@host · ip:port（工作区名前置）
document.getElementById('who').textContent=wsName?server+' · '+wsName:server;
if(wsName)document.title='工作区 '+wsName+' · '+server;
fetch('/api/me').then(r=>r.json()).then(me=>{const s=(me.servers||[]).find(x=>x.name===server);
  if(s){const who=(s.username?s.username+'@':'')+server+' · '+s.host+':'+s.port;
    document.getElementById('titleWho').textContent=(wsName?wsName+'  —  ':'')+who;}}).catch(()=>{});

const proto=location.protocol==='https:'?'wss':'ws';
const ws=new WebSocket(proto+'://'+location.host+'/ws/ssh/'+encodeURIComponent(server)+(wsName?('?ws='+encodeURIComponent(wsName)):''));
ws.onopen=()=>{dot.classList.add('on');sendResize();};
ws.onmessage=e=>term.write(e.data);
ws.onclose=()=>{dot.classList.remove('on');dot.classList.add('off');term.write('\\r\\n\\x1b[2m[devsys] '+(wsName?'已断开 · 工作区仍在后台运行，回门户可重新接入':'连接已关闭')+'\\x1b[0m\\r\\n');};
term.onData(d=>{if(ws.readyState===1)ws.send(JSON.stringify({t:'i',d}));});
function sendResize(){try{fit.fit()}catch(e){}if(ws.readyState===1)ws.send(JSON.stringify({t:'r',c:term.cols,r:term.rows}));}
window.addEventListener('resize',sendResize);
</script></body></html>"""

HOME_HTML = HOME_HTML.replace("__TOKENS__", CIBOL_TOKENS)
TERM_HTML = TERM_HTML.replace("__TOKENS__", CIBOL_TOKENS)
