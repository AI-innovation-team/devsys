"""管理员 API（仅 config.yaml oauth.admins 可访问）。

用户总览 / 邮箱用户增删 / 服务器增删改（运行时真源）/ GitHub 白名单 / 日志。
改白名单或邮箱用户会重启 oauth2（几秒新登录短断，已登录 cookie 不受影响）。
"""
import asyncio
import re

from fastapi import APIRouter, Depends, HTTPException, Request

from .. import audit, oauth_cfg
from ..auth import require_admin
from ..config import ADMINS, DATA
from ..htpasswd import add_email_user, del_email_user, email_users
from ..htpasswd import restart_oauth2
from ..servers import servers, write_servers
from ..storage import load_meta

router = APIRouter()


# ── 用户总览（只读）──────────────────────────────────────────────
@router.get("/api/admin/users")
def admin_users(admin: str = Depends(require_admin)):
    emails = set(email_users())
    gh = set(oauth_cfg.get_github_users())
    configured = {}
    users_dir = DATA / "users"
    if users_dir.exists():
        for d in users_dir.iterdir():
            if d.is_dir():
                configured[d.name] = sorted(load_meta(d.name).keys())
    allu = sorted(gh | emails | set(configured))
    out = [{
        "user": u,
        "kind": "email" if u in emails else "github",
        "is_admin": u in ADMINS,
        "whitelisted": (u in emails) or (u in gh),
        "servers": configured.get(u, []),
    } for u in allu]
    return {"users": out, "admins": sorted(ADMINS)}


# ── 邮箱用户增删（复用 htpasswd 运行时可写闭环）──────────────────
@router.post("/api/admin/users/email")
async def admin_add_email(request: Request, admin: str = Depends(require_admin)):
    body = await request.json()
    email = str(body.get("email", "")).strip()
    pw = str(body.get("password", "")).strip()
    if "@" not in email or email.startswith("@") or email.endswith("@"):
        raise HTTPException(400, "请输入有效邮箱")
    if len(pw) < 8:
        raise HTTPException(400, "密码至少 8 位")
    try:
        add_email_user(email, pw)
        oauth_cfg.sync_htpasswd(bool(email_users()))
        await restart_oauth2()
    except Exception as e:
        raise HTTPException(500, str(e)[:160])
    audit.record(admin, "add_email_user", target=email)
    return {"ok": True}


@router.post("/api/admin/users/email/delete")
async def admin_del_email(request: Request, admin: str = Depends(require_admin)):
    body = await request.json()
    email = str(body.get("email", "")).strip()
    if email not in set(email_users()):
        raise HTTPException(404, "该邮箱用户不存在")
    try:
        del_email_user(email)
        oauth_cfg.sync_htpasswd(bool(email_users()))
        await restart_oauth2()
    except Exception as e:
        raise HTTPException(500, str(e)[:160])
    audit.record(admin, "del_email_user", target=email)
    return {"ok": True}


# ── 服务器增删改（运行时真源，即时生效）─────────────────────────
def _validate_servers(items) -> list:
    if not isinstance(items, list):
        raise HTTPException(400, "servers 必须是列表")
    names = set()
    norm = []
    for s in items:
        name = str(s.get("name", "")).strip()
        host = str(s.get("host", "")).strip()
        if not name or not host:
            raise HTTPException(400, "每台服务器需要 name 和 host")
        if not all(c.isalnum() or c in "-_." for c in name):
            raise HTTPException(400, f"服务器名含非法字符：{name}")
        if name in names:
            raise HTTPException(400, f"服务器名重复：{name}")
        names.add(name)
        try:
            port = int(s.get("port") or 22)
        except (TypeError, ValueError):
            raise HTTPException(400, f"{name} 的端口不是数字")
        item = {"name": name, "host": host, "port": port}
        jump = str(s.get("jump") or "").strip()
        if jump:
            item["jump"] = jump
        norm.append(item)
    for it in norm:  # jump 不能悬空
        if it.get("jump") and it["jump"] not in names:
            raise HTTPException(400, f"{it['name']} 的 jump 指向不存在的服务器：{it['jump']}")
    return norm


@router.get("/api/admin/servers")
def admin_get_servers(admin: str = Depends(require_admin)):
    return {"servers": servers()}


@router.post("/api/admin/servers")
async def admin_save_servers(request: Request, admin: str = Depends(require_admin)):
    body = await request.json()
    items = _validate_servers(body.get("servers", []))
    write_servers(items)
    audit.record(admin, "save_servers", count=len(items))
    return {"ok": True, "servers": items}


# ── GitHub 白名单（原地改 cfg + 重启）───────────────────────────
@router.get("/api/admin/whitelist")
def admin_get_whitelist(admin: str = Depends(require_admin)):
    return {"github_users": oauth_cfg.get_github_users()}


@router.post("/api/admin/whitelist")
async def admin_set_whitelist(request: Request, admin: str = Depends(require_admin)):
    body = await request.json()
    users = []
    for u in body.get("github_users", []):
        u = str(u).strip()
        if u and u not in users:
            users.append(u)
    # 防锁死：GitHub 管理员必须留在白名单，否则改完自己就登录不了
    for a in ADMINS:
        if "@" not in a and a not in users:
            raise HTTPException(400, f"不能移除管理员 {a}（会导致其无法登录）")
    try:
        oauth_cfg.set_github_users(users)
        await restart_oauth2()
    except Exception as e:
        raise HTTPException(500, str(e)[:160])
    audit.record(admin, "set_whitelist", count=len(users))
    return {"ok": True, "github_users": users}


# ── 日志（audit 门户自写；oauth2/portal 走受限免密 journalctl）──
@router.get("/api/admin/logs")
async def admin_logs(admin: str = Depends(require_admin), src: str = "audit", n: int = 30):
    n = max(1, min(int(n), 500))
    if src == "audit":
        return {"src": src, "lines": audit.tail(n)}
    if src in ("oauth2", "portal"):
        unit = "devsys-oauth2" if src == "oauth2" else "devsys-portal"
        # portal 全是 uvicorn 访问日志，要滤掉噪音后取最新 n，故多拉一些
        fetch = max(n * 12, 400) if src == "portal" else n
        p = await asyncio.create_subprocess_exec(
            "sudo", "-n", "journalctl", "-u", unit, "-n", str(fetch),
            "--no-pager", "--output=short-iso",
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
        so, se = await p.communicate()
        if p.returncode != 0:
            raise HTTPException(500, "读取日志失败：" + se.decode()[:140])
        lines = [ln for ln in so.decode().splitlines() if ln.strip()]
        if src == "portal":
            # 滤掉 uvicorn 常规访问日志（成功的 GET/HEAD），只留写操作/错误/服务事件
            lines = [ln for ln in lines if not re.search(r'"(GET|HEAD) [^"]*" (2\d\d|3\d\d)\b', ln)]
        return {"src": src, "text": "\n".join(lines[-n:])}
    raise HTTPException(400, "未知日志源")
