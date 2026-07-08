"""审计日志：append 一行 JSON 到 DATA/audit.log，倒序读尾部。

管理操作(增删用户/改服务器/改白名单)与会话(SSH/VSCode/工作区/上传)都记这里。
"""
import json
import time

from .config import DATA

_LOG = DATA / "audit.log"


def record(actor: str, action: str, **fields) -> None:
    try:
        DATA.mkdir(parents=True, exist_ok=True)
        line = json.dumps({"ts": int(time.time()), "actor": actor, "action": action, **fields}, ensure_ascii=False)
        with open(_LOG, "a") as f:
            f.write(line + "\n")
    except Exception:
        pass  # 审计失败绝不影响主流程


def tail(n: int = 200) -> list:
    if not _LOG.exists():
        return []
    lines = _LOG.read_text().splitlines()[-n:]
    out = []
    for ln in reversed(lines):
        try:
            out.append(json.loads(ln))
        except Exception:
            pass
    return out
