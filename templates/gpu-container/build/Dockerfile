# 开发容器基础镜像。以 CUDA 运行时打底，方便 ML 用途。
# 如果某台机器 CUDA 版本不同，改这里的 tag。
FROM nvidia/cuda:12.4.1-runtime-ubuntu22.04

ARG DEBIAN_FRONTEND=noninteractive

# 换阿里云 apt 源（国内直连快、稳），并移除基础镜像自带的 nvidia cuda apt 源
# （构建时用不到、且 developer.download.nvidia.com 国内可能拉不动，会拖垮 apt update）。
RUN rm -f /etc/apt/sources.list.d/*.list \
	&& sed -i 's|http://archive.ubuntu.com|http://mirrors.aliyun.com|g; s|http://security.ubuntu.com|http://mirrors.aliyun.com|g' /etc/apt/sources.list \
	&& apt-get update && apt-get install -y --no-install-recommends \
		sudo curl wget git vim ca-certificates openssh-client \
		build-essential python3 python3-pip locales \
	&& rm -rf /var/lib/apt/lists/*

RUN locale-gen en_US.UTF-8

# Coder agent 要求以非 root 用户运行；建一个有 sudo 的 coder 用户
RUN useradd --create-home --shell /bin/bash coder \
	&& echo "coder ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/coder \
	&& chmod 0440 /etc/sudoers.d/coder

USER coder
WORKDIR /home/coder
