"""邮箱登录用户表（oauth2-proxy htpasswd，bcrypt）。门户自助改密。

oauth2-proxy 只在启动时读 htpasswd，故改密后需重启 oauth2 服务才生效
（门户以网关用户身份跑，靠一条精确的免密 sudo restart，见 deploy/gateway/sudoers）。
"""
import asyncio

import bcrypt

from .config import HTPASSWD, OAUTH2_SERVICE


def _entries() -> dict:
    if not HTPASSWD.exists():
        return {}
    out = {}
    for line in HTPASSWD.read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#") and ":" in line:
            k, v = line.split(":", 1)
            out[k] = v
    return out


def is_email_user(user: str) -> bool:
    return user in _entries()


def email_users() -> list:
    return sorted(_entries().keys())


def _flush(ents: dict) -> None:
    HTPASSWD.write_text(("\n".join(f"{k}:{v}" for k, v in ents.items()) + "\n") if ents else "")
    HTPASSWD.chmod(0o600)


def set_password(user: str, password: str) -> None:
    ents = _entries()
    if user not in ents:                       # 仅允许改已存在的邮箱账号（申请制不被绕过）
        raise RuntimeError("非邮箱登录账号，无法改密")
    ents[user] = bcrypt.hashpw(password.encode(), bcrypt.gensalt()).decode()
    _flush(ents)


def add_email_user(email: str, password: str) -> None:
    """管理员新增/重置邮箱用户（与 set_password 不同：允许新建）。"""
    ents = _entries()
    ents[email] = bcrypt.hashpw(password.encode(), bcrypt.gensalt()).decode()
    _flush(ents)


def del_email_user(email: str) -> None:
    ents = _entries()
    if email in ents:
        del ents[email]
        _flush(ents)


async def restart_oauth2() -> None:
    p = await asyncio.create_subprocess_exec(
        "sudo", "-n", "systemctl", "restart", OAUTH2_SERVICE,
        stdout=asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.PIPE)
    _, err = await p.communicate()
    if p.returncode != 0:
        raise RuntimeError(f"重启 oauth2 失败：{err.decode()[:140]}")
