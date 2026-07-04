"""每人按需在目标机起自己的 code-server，端口转发回门户，按 /vscode/{server}/ 子路径代理。

home 常是跨机共享 NFS，端口文件与 user-data-dir 按主机名隔离，否则互相打架。
"""
import asyncio

from .ssh import ssh_connect

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

_sessions = {}   # (user, server) -> {"conn", "listener", "port"}
_locks = {}


async def ensure_codeserver(user: str, server: str) -> int:
    key = (user, server)
    sess = _sessions.get(key)
    if sess:
        return sess["port"]
    lock = _locks.setdefault(key, asyncio.Lock())
    async with lock:
        if key in _sessions:
            return _sessions[key]["port"]
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
        _sessions[key] = {"conn": conn, "listener": listener, "port": listener.get_port()}
        return _sessions[key]["port"]


def drop_session(user: str, server: str) -> None:
    sess = _sessions.pop((user, server), None)
    if sess:
        for k in ("listener", "conn"):
            try:
                sess[k].close()
            except Exception:
                pass
