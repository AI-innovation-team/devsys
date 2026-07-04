# 构建带「指定 DNS 服务商插件」的 Caddy。
# 官方 caddy 镜像不含 DNS 插件，用 builder 阶段按需编译进去，
# 这样才能用 DNS-01 方式签证书（不需要 80 端口，绕过未备案域名的 HTTP 拦截）。
# 插件由 CADDY_DNS_MODULE 决定（来自 config.yaml 的 dns.module）。
ARG CADDY_DNS_MODULE=github.com/caddy-dns/alidns

FROM caddy:2-builder AS builder
ARG CADDY_DNS_MODULE
# GOPROXY 默认走国内镜像（cn 优先，回落官方 direct），兼容墙内外
ARG GOPROXY=https://goproxy.cn,https://proxy.golang.org,direct
# 构建时清空宿主注入的代理：容器内 127.0.0.1:7890 不存在，
# 而 goproxy.cn 直连即可，无需代理。
ENV GOPROXY=${GOPROXY} \
    HTTP_PROXY="" HTTPS_PROXY="" http_proxy="" https_proxy="" \
    NO_PROXY="*"
RUN xcaddy build --with ${CADDY_DNS_MODULE}

FROM caddy:2
COPY --from=builder /usr/bin/caddy /usr/bin/caddy
