# 用 xcaddy 把 config.yaml 里指定的 DNS-01 插件编译进 Caddy。
# 国内构建：GOPROXY 走 goproxy.cn，避免 proxy.golang.org 卡死。
ARG CADDY_DNS_MODULE=github.com/caddy-dns/alidns

FROM caddy:2-builder AS build
ARG CADDY_DNS_MODULE
ENV GOPROXY=https://goproxy.cn,direct
ENV GOSUMDB=off
# docker build 会自动把宿主的翻墙代理注入为 HTTP(S)_PROXY，但 buildkit 构建容器非 host 网络、
# 127.0.0.1:7890 不通；goproxy.cn 国内直连即可，故清掉代理再编译，避免受宿主代理开关影响。
RUN unset HTTP_PROXY HTTPS_PROXY http_proxy https_proxy ALL_PROXY all_proxy; \
    xcaddy build --with ${CADDY_DNS_MODULE}

FROM caddy:2
COPY --from=build /usr/bin/caddy /usr/bin/caddy
