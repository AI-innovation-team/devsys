"""文档：目录结构即层级。维护 docs/ 下的 .md 文件与文件夹即可（即时生效）。

约定（均可选，优雅降级）：
  - 排序：名字的 `NN-` 数字前缀作排序键，显示时去除；无前缀者排后。
  - 标题：frontmatter `title:` > 首个 `# H1` > 文件名美化。
  - 分组：文件夹内的 `_index.md`/`index.md`/`README.md` 作为该组的着陆页与标题；
          没有则用文件夹名。
  - frontmatter：可选 `--- key: value ---`，渲染前剥离。

slug = 相对 docs 根的 posix 路径（去扩展名），如 `10-servers/01-turing`。
内容按“发现到的文件”白名单匹配，杜绝路径穿越。
"""
import re
from pathlib import Path

from .config import DOCS

INDEX_NAMES = ("_index.md", "index.md", "readme.md")
MAX_DEPTH = 8
_ORDER_RE = re.compile(r"^(\d+)[-_.](.*)$")


def _read(p: Path) -> str:
    try:
        return p.read_text(encoding="utf-8")
    except Exception:
        return ""


def _split_frontmatter(text: str):
    """返回 (meta: dict, body: str)。仅解析简单的 key: value 行。"""
    if not text.startswith("---"):
        return {}, text
    lines = text.splitlines()
    for i in range(1, len(lines)):
        if lines[i].strip() == "---":
            meta = {}
            for ln in lines[1:i]:
                if ":" in ln:
                    k, _, v = ln.partition(":")
                    meta[k.strip().lower()] = v.strip().strip('"').strip("'")
            body = "\n".join(lines[i + 1:]).lstrip("\n")
            return meta, body
    return {}, text


def _order_name(name: str):
    """拆出 (排序序号, 去前缀名)。无数字前缀返回大序号排后。"""
    m = _ORDER_RE.match(name)
    if m:
        return int(m.group(1)), m.group(2)
    return 10 ** 6, name


def _humanize(stem: str) -> str:
    _, clean = _order_name(stem)
    return clean.replace("-", " ").replace("_", " ").strip() or stem


def _title_of(text: str, fallback: str) -> str:
    meta, body = _split_frontmatter(text)
    if meta.get("title"):
        return meta["title"]
    for ln in body.splitlines():
        s = ln.strip()
        if s.startswith("# "):
            return s[2:].strip()
    return fallback


def _slug(p: Path) -> str:
    return p.relative_to(DOCS).with_suffix("").as_posix()


def _index_file(d: Path):
    for e in d.iterdir():
        if e.is_file() and e.name.lower() in INDEX_NAMES:
            return e
    return None


def _walk(d: Path, depth: int) -> list:
    if depth > MAX_DEPTH:
        return []
    nodes = []
    for e in d.iterdir():
        if e.name.startswith("."):
            continue
        if e.is_dir() and not e.is_symlink():
            idx = _index_file(e)
            children = _walk(e, depth + 1)
            if not children and idx is None:
                continue  # 空文件夹跳过
            order, _ = _order_name(e.name)
            title = _title_of(_read(idx), _humanize(e.name)) if idx else _humanize(e.name)
            node = {"type": "group", "title": title, "path": e.relative_to(DOCS).as_posix(),
                    "children": children, "_order": order}
            if idx is not None:
                node["slug"] = _slug(idx)  # 组标题可点击打开着陆页
            nodes.append(node)
        elif e.is_file() and e.suffix.lower() == ".md" and e.name.lower() not in INDEX_NAMES:
            order, _ = _order_name(e.stem)
            nodes.append({"type": "doc", "slug": _slug(e),
                          "title": _title_of(_read(e), _humanize(e.stem)), "_order": order})
    nodes.sort(key=lambda n: (n["_order"], n["title"].lower()))
    for n in nodes:
        n.pop("_order", None)
    return nodes


def doc_tree() -> list:
    if not DOCS.exists():
        return []
    return _walk(DOCS, 0)


def _all_docs() -> dict:
    """slug -> Path 的白名单（含 _index）。"""
    out = {}
    if not DOCS.exists():
        return out
    for p in DOCS.rglob("*.md"):
        rel = p.relative_to(DOCS)
        if p.is_file() and not any(part.startswith(".") for part in rel.parts):
            out[_slug(p)] = p
    return out


def doc_path(slug: str):
    return _all_docs().get(slug)


def doc_view(p: Path):
    """返回 (title, body)：剥离 frontmatter 后的正文。"""
    text = _read(p)
    _, body = _split_frontmatter(text)
    return _title_of(text, _humanize(p.stem)), body
