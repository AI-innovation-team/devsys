#!/usr/bin/env python3
"""回拉：把网关运行时真源（管理界面改过的 servers / GitHub 白名单）文本级回填
本地 config.yaml，保持配置可版本化。只替换 servers: 数据行 和 github_users 行，
保住全文中文注释（devsysconf.py 是只读 YAML 加载器、无 dump，故不能重序列化）。

用法：python3 scripts/pull.py <servers.json> <oauth2-proxy.cfg> <config.yaml>
"""
import json
import re
import sys


def servers_block(servers_json: str) -> str:
    items = json.load(open(servers_json))
    lines = []
    for s in items:
        parts = [f"name: {s['name']}", f"host: {s['host']}", f"port: {int(s.get('port', 22))}"]
        if s.get("jump"):
            parts.append(f"jump: {s['jump']}")
        lines.append("  - { " + ", ".join(parts) + " }")
    return "\n".join(lines)


def github_users(cfg_text: str) -> list:
    m = re.search(r'^github_users\s*=\s*\[(.*)\]', cfg_text, re.M)
    if not m:
        return []
    return [u.strip().strip('"') for u in m.group(1).split(",") if u.strip().strip('"')]


def main(servers_json: str, oauth_cfg: str, config_path: str) -> None:
    text = open(config_path).read()

    # servers: 段——替换连续的 `  - {...}` 数据行（保留 servers: 标题与其后注释/示例）
    new_block = servers_block(servers_json)
    text, n = re.subn(r'(^servers:\n)(?:[ \t]*-[ \t]*\{[^\n]*\}\n)+',
                      lambda m: m.group(1) + new_block + "\n", text, flags=re.M)
    if n == 0:
        sys.exit("✗ 没找到 config.yaml 的 servers: 数据段")

    # github_users 行（内联 [...]，保留缩进）
    gh = github_users(open(oauth_cfg).read())
    gh_line = "  github_users: [" + ", ".join(gh) + "]"
    text, n2 = re.subn(r'^  github_users:.*$', lambda m: gh_line, text, flags=re.M)
    if n2 == 0:
        sys.exit("✗ 没找到 config.yaml 的 github_users 行")

    open(config_path, "w").write(text)
    print(f"✓ 回填完成：{len(json.load(open(servers_json)))} 台服务器，{len(gh)} 个 GitHub 用户")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], sys.argv[3])
