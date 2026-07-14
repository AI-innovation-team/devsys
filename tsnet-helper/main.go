// devsys tsnet sidecar —— 把一个 tailnet 节点嵌进 app,零系统依赖(不装 Tailscale)。
//
// 用官方 tsnet 库,在**用户态**跑一个 tailnet 节点。两个方向都覆盖:
//
//   出站(消费侧 · 我连别人):开一个本地 SOCKS5,russh 走它 → 经 tailnet 到达
//     队友内网的机器。app 一开就能连,零系统依赖。
//
//   入站(贡献侧 · 队友连我):tsnet Listen tailnet 的 :22 → 把连接代理到本机
//     127.0.0.1:22(本地 sshd)。于是队友能 SSH 进我这台机 —— 但 **app 得开着**
//     (用户态节点随进程存亡),这是内建 sidecar 的固有取舍。
//
// 与 Rust 侧用「行分隔 JSON」通信:stdout 报状态,stdin 收指令(暂只需启动参数)。
// 状态目录由 --dir 指定(存 tsnet 身份,重启复用同一节点)。
package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net"
	"os"
	"strings"
	"time"

	"tailscale.com/tsnet"
)

type status struct {
	State   string `json:"state"`             // starting | running | error
	Backend string `json:"backend,omitempty"` // tsnet backend 状态(NeedsLogin/Running…)
	IP      string `json:"ip,omitempty"`      // 本节点 tailnet IPv4
	Name    string `json:"name,omitempty"`    // 本节点在 tailnet 里的名字
	AuthURL string `json:"auth_url,omitempty"`// 需要登录时的 URL(没填 authkey 时)
	SOCKS   string `json:"socks,omitempty"`   // 出站 SOCKS5 监听地址
	Ingress bool   `json:"ingress,omitempty"` // 入站(:22 代理到本机 sshd)是否已开
	Err     string `json:"error,omitempty"`
}

func emit(s status) {
	b, _ := json.Marshal(s)
	fmt.Println(string(b))
	os.Stdout.Sync()
}

func main() {
	var (
		dir       = flag.String("dir", "", "tsnet 状态目录(存节点身份)")
		hostname  = flag.String("hostname", "devsys", "本节点在 tailnet 的名字")
		authkey   = flag.String("authkey", "", "预授权 key(可选;不填则走浏览器登录)")
		socksAddr = flag.String("socks", "127.0.0.1:1055", "出站 SOCKS5 监听")
		ingress   = flag.Bool("ingress", false, "开入站:tailnet:22 → 本机 sshd")
		sshdPort  = flag.Int("sshd-port", 22, "本机 sshd 端口(入站代理目标)")
	)
	flag.Parse()

	if *dir == "" {
		emit(status{State: "error", Err: "--dir 必填"})
		os.Exit(1)
	}

	emit(status{State: "starting"})

	srv := &tsnet.Server{
		Dir:      *dir,
		Hostname: *hostname,
		AuthKey:  *authkey,
		// tsnet 日志很吵,且会污染我们经 stdout 的 JSON 协议 —— 两条日志路都丢弃。
		Logf:     func(string, ...any) {},
		UserLogf: func(string, ...any) {},
	}
	defer srv.Close()

	ctx := context.Background()
	go pollStatus(ctx, srv, *socksAddr, *ingress)

	// ── 出站 SOCKS5 ──
	// 先起监听（不等 Up）：tsnet 的 Dial 会在节点就绪前排队，节点一 Running 就能拨出去。
	// 若等 Up() 返回再监听，未登录节点会永远停在 NeedsLogin → SOCKS 端口永不监听（连接被拒）。
	if *socksAddr != "" {
		ln, err := net.Listen("tcp", *socksAddr)
		if err != nil {
			emit(status{State: "error", Err: "SOCKS 监听失败: " + err.Error()})
			os.Exit(1)
		}
		go serveSocks(ln, srv)
	}

	// ── 入站:tailnet:sshdPort → 本机 sshd ──
	if *ingress {
		ln, err := srv.Listen("tcp", fmt.Sprintf(":%d", *sshdPort))
		if err != nil {
			emit(status{State: "error", Err: "入站监听失败: " + err.Error()})
		} else {
			go proxyIngress(ln, *sshdPort)
		}
	}

	// Up 在后台推进登录（NeedsLogin 时会阻塞到浏览器授权完成）。不阻塞主流程。
	go func() {
		if _, err := srv.Up(ctx); err != nil {
			emit(status{State: "error", Err: "tsnet 启动失败: " + err.Error()})
		}
	}()

	// 保活:随 stdin 关闭而退出(Rust 侧 kill 掉 sidecar 时 stdin EOF)。
	io.Copy(io.Discard, os.Stdin)
}

// 周期性上报节点状态(IP / 名字 / 需要登录的 URL)。
func pollStatus(ctx context.Context, srv *tsnet.Server, socks string, ingress bool) {
	var last string // 上次报过的关键状态指纹,变了才再报(避免刷屏)
	for i := 0; i < 3600; i++ {
		lc, err := srv.LocalClient()
		if err == nil {
			if st, err := lc.Status(ctx); err == nil && st != nil {
				s := status{
					State:   "running",
					Backend: st.BackendState,
					SOCKS:   socks,
					Ingress: ingress,
				}
				if st.Self != nil {
					s.Name = strings.TrimSuffix(st.Self.DNSName, ".")
					for _, ip := range st.Self.TailscaleIPs {
						if ip.Is4() {
							s.IP = ip.String()
							break
						}
					}
				}
				if st.AuthURL != "" {
					s.AuthURL = st.AuthURL
				}
				// 指纹含 backend + IP + authURL —— 任一变化都上报(修掉之前 IP 为空时漏报 authURL)。
				fp := st.BackendState + "|" + s.IP + "|" + s.AuthURL
				if fp != last {
					emit(s)
					last = fp
				}
				if st.BackendState == "Running" && s.IP != "" {
					time.Sleep(10 * time.Second) // 稳定后放慢心跳
					continue
				}
			}
		}
		time.Sleep(1 * time.Second)
	}
}

// 把入站 tailnet 连接透明代理到本机 sshd。
func proxyIngress(ln net.Listener, sshdPort int) {
	for {
		conn, err := ln.Accept()
		if err != nil {
			return
		}
		go func(c net.Conn) {
			defer c.Close()
			up, err := net.DialTimeout("tcp", fmt.Sprintf("127.0.0.1:%d", sshdPort), 5*time.Second)
			if err != nil {
				return // 本机没开 sshd → 队友连不进来(已在 UI 提示)
			}
			defer up.Close()
			go io.Copy(up, c)
			io.Copy(c, up)
		}(conn)
	}
}
