// 极简 SOCKS5 服务端(仅 CONNECT,无认证)—— 监听在 127.0.0.1,只给本机 app 用。
// 收到的连接经 tsnet.Dial 拨出去,于是 russh over SOCKS 就走上了 tailnet。
package main

import (
	"context"
	"encoding/binary"
	"fmt"
	"io"
	"net"

	"tailscale.com/tsnet"
)

func serveSocks(ln net.Listener, srv *tsnet.Server) {
	for {
		c, err := ln.Accept()
		if err != nil {
			return
		}
		go handleSocks(c, srv)
	}
}

func handleSocks(c net.Conn, srv *tsnet.Server) {
	defer c.Close()

	// ── 握手:版本 + 方法协商 ──
	buf := make([]byte, 262)
	if _, err := io.ReadFull(c, buf[:2]); err != nil || buf[0] != 0x05 {
		return
	}
	nmethods := int(buf[1])
	if _, err := io.ReadFull(c, buf[:nmethods]); err != nil {
		return
	}
	// 回:版本 5,方法 0(无需认证)
	if _, err := c.Write([]byte{0x05, 0x00}); err != nil {
		return
	}

	// ── 请求:VER CMD RSV ATYP ──
	if _, err := io.ReadFull(c, buf[:4]); err != nil {
		return
	}
	if buf[1] != 0x01 { // 只支持 CONNECT
		c.Write([]byte{0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0})
		return
	}
	atyp := buf[3]

	var host string
	switch atyp {
	case 0x01: // IPv4
		if _, err := io.ReadFull(c, buf[:4]); err != nil {
			return
		}
		host = net.IP(buf[:4]).String()
	case 0x03: // 域名
		if _, err := io.ReadFull(c, buf[:1]); err != nil {
			return
		}
		l := int(buf[0])
		if _, err := io.ReadFull(c, buf[:l]); err != nil {
			return
		}
		host = string(buf[:l])
	case 0x04: // IPv6
		if _, err := io.ReadFull(c, buf[:16]); err != nil {
			return
		}
		host = net.IP(buf[:16]).String()
	default:
		c.Write([]byte{0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0})
		return
	}

	if _, err := io.ReadFull(c, buf[:2]); err != nil {
		return
	}
	port := binary.BigEndian.Uint16(buf[:2])
	target := net.JoinHostPort(host, fmt.Sprintf("%d", port))

	// ── 经 tailnet 拨出去 ──
	up, err := srv.Dial(context.Background(), "tcp", target)
	if err != nil {
		c.Write([]byte{0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0}) // 连接被拒
		return
	}
	defer up.Close()

	// 成功应答(绑定地址填 0)
	if _, err := c.Write([]byte{0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0}); err != nil {
		return
	}

	go io.Copy(up, c)
	io.Copy(c, up)
}
