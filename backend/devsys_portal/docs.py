"""文档：维护 docs/*.md 即可（即时生效）。标题取首个 `# H1`，排序按文件名。"""
from pathlib import Path

from .config import DOCS


def doc_files() -> list:
    if not DOCS.exists():
        return []
    return sorted((p for p in DOCS.glob("*.md") if p.is_file()), key=lambda p: p.name)


def doc_title(p: Path) -> str:
    try:
        for line in p.read_text(encoding="utf-8").splitlines():
            s = line.strip()
            if s.startswith("# "):
                return s[2:].strip()
    except Exception:
        pass
    return p.stem
