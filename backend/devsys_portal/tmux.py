"""工作区 = tmux 持久会话。用默认 socket，与用户手动 `tmux ls` 同一批会话。

接入采用 `new-session -A -D`（有则接入并踢掉旧连接、无则新建），断开只 detach，会话常驻。
"""
import asyncio
import re

from .ssh import ssh_connect

WS_NAME = re.compile(r"^[A-Za-z0-9_.][A-Za-z0-9_.-]{0,63}$")  # 新建：友好名，不以 - 开头
TMUX = "tmux"
LIST_FMT = "#{session_name}|#{session_created}|#{session_windows}|#{session_attached}|#{session_activity}"


def ok_name(n: str) -> bool:
    """接入/关闭既有会话：名字来自 tmux 本身，可能较随意；只挡换行与超长，命令层用 shlex 转义。"""
    return bool(n) and "\n" not in n and "\r" not in n and len(n) <= 128


async def run(user: str, server: str, cmd: str, timeout: float = 12.0):
    conn = await asyncio.wait_for(ssh_connect(user, server), timeout)
    try:
        r = await asyncio.wait_for(conn.run(cmd, check=False), timeout)
        return r.exit_status, r.stdout, r.stderr
    finally:
        conn.close()


def parse_sessions(out: str) -> list:
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


def attach_cmd(name: str) -> str:
    import shlex
    return f"{TMUX} new-session -A -D -s {shlex.quote(name)}"
