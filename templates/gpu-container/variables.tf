variable "machines" {
  type        = list(string)
  description = "可选的内网机器清单，必须和各 node 的 NODE_NAME 一一对应"
  default     = ["node1"]
}
