#!/usr/bin/env bash
# 在网关机上运行：添加/更新一个邮箱登录用户，写入 htpasswd(bcrypt) 并重启 oauth2。
#   bash add-email-user.sh <email> [password]        # 不给密码则随机生成并打印
#   bash add-email-user.sh --del <email>             # 删除某用户
set -euo pipefail
HT="$HOME/gateway/oauth2/htpasswd"
touch "$HT"

if [ "${1:-}" = "--del" ]; then
  email="${2:?用法: add-email-user.sh --del <email>}"
  awk -F: -v e="$email" '$1!=e' "$HT" > "$HT.tmp" && mv "$HT.tmp" "$HT"
  sudo systemctl restart devsys-oauth2
  echo "✓ 已删除：$email"; exit 0
fi

email="${1:?用法: add-email-user.sh <email> [password]}"
pw="${2:-}"
# 不指定密码则用统一临时密码（无特殊字符，避免中文输入法全角/自动填充问题）。
# 提醒用户登录后到「设置 → 账户」尽快改。
[ -n "$pw" ] || pw='modifyme2026'
# bcrypt：优先用后端 venv（版本无关），退回系统 python crypt
PY="$HOME/gateway/backend/.venv/bin/python"; [ -x "$PY" ] || PY=python3
hash=$("$PY" -c 'import sys,bcrypt;print(bcrypt.hashpw(sys.argv[1].encode(),bcrypt.gensalt()).decode())' "$pw" 2>/dev/null) \
  || hash=$(python3 -c 'import crypt,sys;print(crypt.crypt(sys.argv[1],crypt.mksalt(crypt.METHOD_BLOWFISH)))' "$pw")

awk -F: -v e="$email" '$1!=e' "$HT" > "$HT.tmp"       # 去掉同邮箱旧行
echo "${email}:${hash}" >> "$HT.tmp"
mv "$HT.tmp" "$HT"
chmod 600 "$HT"
# 确保 oauth2 配置启用了邮箱登录（install.sh 在无用户时会摘除，这里补回）
cfg="$HOME/gateway/oauth2/oauth2-proxy.cfg"
grep -q "^htpasswd_file" "$cfg" || printf 'htpasswd_file = "%s"\ndisplay_htpasswd_form = true\n' "$HT" >> "$cfg"
sudo systemctl restart devsys-oauth2
echo "✓ 已添加/更新：$email"
echo "  密码：$pw"
