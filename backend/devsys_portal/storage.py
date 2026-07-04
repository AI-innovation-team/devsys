"""每个用户自设的连接凭据：明文元数据（用户名/认证方式）+ 加密密钥/密码。

布局：DATA/users/<gh用户>/meta.json           # {server: {username, auth}}
      DATA/users/<gh用户>/secrets/<server>.enc  # Fernet 密文（密码或私钥）
"""
import json
from pathlib import Path
from typing import Optional

from .config import DATA
from .crypto import fernet


def user_dir(user: str) -> Path:
    d = DATA / "users" / user
    (d / "secrets").mkdir(parents=True, exist_ok=True)
    return d


def load_meta(user: str) -> dict:
    f = user_dir(user) / "meta.json"
    return json.loads(f.read_text()) if f.exists() else {}


def save_meta(user: str, meta: dict) -> None:
    (user_dir(user) / "meta.json").write_text(json.dumps(meta, indent=2))


def secret_path(user: str, server: str) -> Path:
    return user_dir(user) / "secrets" / f"{server}.enc"


def write_secret(user: str, server: str, plaintext: str) -> None:
    p = secret_path(user, server)
    p.write_bytes(fernet().encrypt(plaintext.encode()))
    p.chmod(0o600)


def read_secret(user: str, server: str) -> Optional[str]:
    p = secret_path(user, server)
    return fernet().decrypt(p.read_bytes()).decode() if p.exists() else None
