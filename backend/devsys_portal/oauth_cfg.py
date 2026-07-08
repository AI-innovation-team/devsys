"""原地编辑 oauth2-proxy.cfg —— 只改 github_users / htpasswd 两处，绝不重生成整个 cfg。

门户机上没有 .env/.secrets，无法重新渲染完整 cfg；若重生成会丢 client_secret /
cookie_secret，导致 oauth2-proxy 启动失败或所有已登录用户 cookie 失效。故一律行内替换。
"""
import re

from .config import OAUTH2_CFG


def _read() -> str:
    return OAUTH2_CFG.read_text()


def _write(text: str) -> None:
    OAUTH2_CFG.write_text(text)
    OAUTH2_CFG.chmod(0o600)


def get_github_users() -> list:
    m = re.search(r'^github_users\s*=\s*\[(.*)\]\s*$', _read(), re.M)
    if not m:
        return []
    return [u.strip().strip('"') for u in m.group(1).split(",") if u.strip().strip('"')]


def set_github_users(users: list) -> None:
    line = "github_users = [ " + ", ".join(f'"{u}"' for u in users) + " ]"
    text = _read()
    if re.search(r'^github_users\s*=', text, re.M):
        text = re.sub(r'^github_users\s*=.*$', line, text, count=1, flags=re.M)
    else:
        text = text.rstrip("\n") + "\n" + line + "\n"
    _write(text)


def sync_htpasswd(has_users: bool) -> None:
    """有邮箱用户则确保 htpasswd_file/display 行在，无则摘除（oauth2-proxy 拒绝空 htpasswd 会崩）。"""
    text = _read()
    has_line = bool(re.search(r'^htpasswd_file\s*=', text, re.M))
    if has_users and not has_line:
        ht = str(OAUTH2_CFG.parent / "htpasswd")
        text = text.rstrip("\n") + f'\nhtpasswd_file = "{ht}"\ndisplay_htpasswd_form = true\n'
        _write(text)
    elif not has_users and has_line:
        text = re.sub(r'^htpasswd_file\s*=.*$\n?', '', text, flags=re.M)
        text = re.sub(r'^display_htpasswd_form\s*=.*$\n?', '', text, flags=re.M)
        _write(text)
