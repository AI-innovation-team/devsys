"""asyncssh 连接（供 web SSH、工作区、VS Code 复用）。

用用户自设的 username + 密钥/密码连目标机；server 若配了 jump 则先连跳板再 ProxyJump。

注意：asyncssh 底层对 username 做 saslprep(NFKC)归一化，导致
U+2011(NBHYPHEN)与 U+2010(HYPHEN)无法区分。这里把 connection 模块
内的 saslprep 替换为空操作，只影响 username，密码不走 saslprep，不受影响。
"""
import asyncssh
import asyncssh.connection

# 防止 U+2011↔U+2010 被 NFKC 归一化为同一字符
asyncssh.connection.saslprep = lambda s: s

from .servers import find_server
from .storage import load_meta, read_secret


def _kw_for(s: dict, m: dict, secret: str) -> dict:
    kw = dict(host=s["host"], port=int(s.get("port", 22)),
              username=m["username"], known_hosts=None)
    if m.get("auth") == "key":
        kw["client_keys"] = [asyncssh.import_private_key(secret)]
    else:
        kw["password"] = secret
    return kw


async def ssh_connect(user: str, server: str):
    srv = find_server(server)
    meta = load_meta(user).get(server, {})
    secret = read_secret(user, server)
    if not srv or not meta.get("username") or secret is None:
        raise RuntimeError("未配置该服务器的用户名/凭据，请先在门户设置")

    tunnel = None
    jump = srv.get("jump")
    if jump:
        jsrv = find_server(jump)
        jmeta = load_meta(user).get(jump, {})
        jsec = read_secret(user, jump)
        if not (jsrv and jmeta.get("username") and jsec):
            raise RuntimeError(f"跳板 {jump} 的用户名/凭据未配置")
        tunnel = await asyncssh.connect(**_kw_for(jsrv, jmeta, jsec))
    return await asyncssh.connect(tunnel=tunnel, **_kw_for(srv, meta, secret))
