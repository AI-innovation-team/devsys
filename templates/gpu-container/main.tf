terraform {
  required_providers {
    coder  = { source = "coder/coder" }
    docker = { source = "kreuzwerker/docker" }
  }
}

# 单节点部署：Coder OSS 用内置置备器，在控制面机器本地 Docker 上建容器，
# 不需要「选机器」参数/标签。多机需企业版外部置备器或远程 Docker host（阶段二）。

data "coder_workspace" "me" {}
data "coder_workspace_owner" "me" {}

locals {
  username = data.coder_workspace_owner.me.name
}

# ── Coder agent：跑在容器里，负责连回 Coder Server、提供终端等 ──────
resource "coder_agent" "main" {
  arch           = "amd64"
  os             = "linux"
  startup_script = <<-EOT
    set -e
    echo "workspace ready"
  EOT

  # 容器里已经能用「终端」按钮（web terminal 是 agent 自带的）。
  # GPU 状态展示在 workspace 页面上，方便确认直通生效。
  metadata {
    display_name = "GPU"
    key          = "gpu"
    script       = "nvidia-smi --query-gpu=utilization.gpu,memory.used,memory.total --format=csv,noheader || echo 'no gpu'"
    interval     = 10
    timeout      = 5
  }
}

# 浏览器版 VS Code（官方模块，自动装好 code-server 并加一个「VS Code」按钮）
module "code-server" {
  source   = "registry.coder.com/modules/code-server/coder"
  version  = "~> 1.0"
  agent_id = coder_agent.main.id
}

# ── 持久化 home：workspace 停了再开数据不丢 ───────────────────────
resource "docker_volume" "home" {
  name = "coder-${data.coder_workspace.me.id}-home"
  # 避免误删用户数据
  lifecycle { ignore_changes = all }
}

# ── 构建镜像（构建发生在用户选的那台内网机器本地）───────────────
resource "docker_image" "main" {
  name = "coder-devsys-${data.coder_workspace.me.id}"
  build {
    context = "./build"
  }
  triggers = { dir_sha1 = sha1(join("", [for f in fileset(path.module, "build/*") : filesha1("${path.module}/${f}")])) }
}

# ── 用户容器 ─────────────────────────────────────────────────────
resource "docker_container" "workspace" {
  count = data.coder_workspace.me.start_count  # 停止时销毁容器，home 卷保留
  image = docker_image.main.name
  name  = "coder-${local.username}-${lower(data.coder_workspace.me.name)}"
  # agent 的启动脚本作为容器入口
  entrypoint = ["sh", "-c", coder_agent.main.init_script]
  env = [
    "CODER_AGENT_TOKEN=${coder_agent.main.token}",
    "NVIDIA_VISIBLE_DEVICES=all",
    "NVIDIA_DRIVER_CAPABILITIES=all",
  ]

  # ── GPU 直通 ──
  # 需要内网机器已装 nvidia-container-toolkit，并把 nvidia 注册为 docker runtime。
  # 若只想给部分卡，把 NVIDIA_VISIBLE_DEVICES 改成 "0,1" 之类。
  runtime = "nvidia"

  # 持久化 home
  volumes {
    container_path = "/home/coder"
    volume_name    = docker_volume.home.name
    read_only      = false
  }

  # 把宿主机的共享数据目录挂进容器（按需改路径；没有就删掉这块）
  volumes {
    host_path      = "/data"
    container_path = "/data"
    read_only      = false
  }
}
