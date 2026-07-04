#!/usr/bin/env python3
"""极简 YAML 子集加载器（纯标准库，Mac 上无需 PyYAML）。

支持 devsys config.yaml 所需的结构：
  key: scalar
  key: {a: 1, b: 2}          内联 map
  key: [a, b, c]             内联 list
  key:\n  nested: map        缩进 map
  key:\n  - {..}\n  - x      缩进 list（元素可为内联 map / 标量）
注释：整行或空格后的 #。
"""
import sys


def _strip_comment(s: str) -> str:
    out, prev = [], " "
    for ch in s:
        if ch == "#" and prev in " \t":
            break
        out.append(ch)
        prev = ch
    return "".join(out)


def _unquote(v: str):
    v = v.strip()
    if len(v) >= 2 and v[0] in "\"'" and v[-1] == v[0]:
        return v[1:-1]
    if v == "":
        return ""
    return v


def _inline_list(s: str) -> list:
    s = s.strip()[1:-1]
    return [_unquote(x) for x in s.split(",")] if s.strip() else []


def _inline_map(s: str) -> dict:
    s = s.strip()[1:-1]
    d = {}
    for part in s.split(","):
        if ":" in part:
            k, v = part.split(":", 1)
            d[k.strip()] = _scalar(v)
    return d


def _scalar(s: str):
    s = s.strip()
    if s.startswith("{"):
        return _inline_map(s)
    if s.startswith("["):
        return _inline_list(s)
    return _unquote(s)


def _parse(lines, i, indent):
    """返回 (值, 下一行号)。值为 dict 或 list。"""
    if lines[i][1].startswith("- "):
        items = []
        while i < len(lines) and lines[i][0] == indent and lines[i][1].startswith("- "):
            item = lines[i][1][2:].strip()
            if item:
                items.append(_scalar(item))
                i += 1
            else:
                sub, i = _parse(lines, i + 1, lines[i + 1][0])
                items.append(sub)
        return items, i
    d = {}
    while i < len(lines) and lines[i][0] == indent and not lines[i][1].startswith("- "):
        key, _, rest = lines[i][1].partition(":")
        key, rest = key.strip(), rest.strip()
        i += 1
        if rest:
            d[key] = _scalar(rest)
        elif i < len(lines) and lines[i][0] > indent:
            d[key], i = _parse(lines, i, lines[i][0])
        else:
            d[key] = None
    return d, i


def load(path: str) -> dict:
    lines = []
    for raw in open(path, encoding="utf-8"):
        s = _strip_comment(raw.rstrip("\n"))
        if s.strip():
            lines.append((len(s) - len(s.lstrip()), s.strip()))
    if not lines:
        return {}
    val, _ = _parse(lines, 0, 0)
    return val


if __name__ == "__main__":
    import json
    print(json.dumps(load(sys.argv[1]), ensure_ascii=False, indent=2))
