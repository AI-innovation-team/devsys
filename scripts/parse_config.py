#!/usr/bin/env python3
# 只用标准库解析 devsys 的 config.yaml（固定 schema，不依赖 PyYAML）。
# 输出可被 shell eval 的 KEY=VALUE 行。
import sys, shlex

def strip_comment(s):
    out = []
    for i, ch in enumerate(s):
        if ch == '#' and (i == 0 or s[i-1] in ' \t'):
            break
        out.append(ch)
    return ''.join(out)

def unquote(v):
    v = v.strip()
    if len(v) >= 2 and v[0] in '"\'' and v[-1] == v[0]:
        v = v[1:-1]
    return v

def parse_inline_map(s):
    # { name: turing, ssh: turing }
    s = s.strip().lstrip('{').rstrip('}')
    d = {}
    for part in s.split(','):
        if ':' in part:
            k, v = part.split(':', 1)
            d[k.strip()] = unquote(v)
    return d

def main(path):
    lines = []
    for raw in open(path):
        line = strip_comment(raw.rstrip('\n'))
        if line.strip() == '':
            continue
        lines.append(line)

    data, i, n = {}, 0, len(lines)
    while i < n:
        line = lines[i]
        if line[0] in ' \t':  # 顶层键不缩进；跳过异常缩进
            i += 1; continue
        key, _, rest = line.partition(':')
        key = key.strip(); rest = rest.strip()
        if rest and not rest.startswith('{'):
            data[key] = unquote(rest)          # 顶层标量
            i += 1
        else:
            i += 1
            if key == 'nodes':                 # 列表
                items = []
                while i < n and lines[i].lstrip().startswith('-'):
                    item = lines[i].lstrip()[1:].strip()
                    items.append(parse_inline_map(item) if item.startswith('{') else {})
                    i += 1
                data[key] = items
            else:                              # 嵌套 map（relay / control_plane）
                m = {}
                while i < n and lines[i][0] in ' \t' and not lines[i].lstrip().startswith('-'):
                    k, _, v = lines[i].strip().partition(':')
                    m[k.strip()] = unquote(v)
                    i += 1
                data[key] = m
    return data

def emit(k, v):
    print(f'{k}={shlex.quote(str(v))}')

if __name__ == '__main__':
    c = main(sys.argv[1])
    emit('DOMAIN', c['domain']); emit('PUBLIC_PORT', c['public_port']); emit('ACME_EMAIL', c['acme_email'])
    emit('CONTAINER_PROXY', c.get('container_proxy', ''))
    emit('CONTROL_PLANE_LAN', c.get('control_plane_lan', ''))
    r = c['relay']
    emit('RELAY_SSH', r['ssh']); emit('RELAY_USER', r['user']); emit('RELAY_HOST', r['host']); emit('RELAY_PORT', r.get('port', 22))
    emit('CP_SSH', c['control_plane']['ssh'])
    emit('CADDY_DNS_MODULE', c['dns']['module'])   # xcaddy 编译用的插件
    ns = c.get('nodes', [])
    print('NODES=(' + ' '.join(shlex.quote(f"{x['name']}:{x['ssh']}") for x in ns) + ')')
    print('NODE_NAMES=(' + ' '.join(shlex.quote(x['name']) for x in ns) + ')')
