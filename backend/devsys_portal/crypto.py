"""凭据落盘用 Fernet 对称加密。主密钥自动生成、600 权限。"""
from cryptography.fernet import Fernet

from .config import DATA


def fernet() -> Fernet:
    kf = DATA / "portal.key"
    if not kf.exists():
        DATA.mkdir(parents=True, exist_ok=True)
        kf.write_bytes(Fernet.generate_key())
        kf.chmod(0o600)
    return Fernet(kf.read_bytes())
