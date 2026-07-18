import { useEffect, useRef, useState } from "react";

import { TermView } from "../components/TermView";
import { Icon } from "../icons";
import "../styles/terminal.css";

// 全屏终端页（Servers 屏「SSH」按钮走这条老路）。
// 终端本体已抽成 TermView（components/TermView.tsx），这里只剩「整页外壳」：
// 固定满屏容器 + 顶栏（返回/品牌/连接指示）+ 软键盘贴合。工作区的 pane 复用同一个 TermView。
export function Terminal({ server, ws, onBack }: { server: string; ws: string; onBack?: () => void }) {
  const page = useRef<HTMLDivElement>(null);
  const [conn, setConn] = useState<null | boolean>(null);
  const who = ws ? server + " · " + ws : server;

  // 让辅助键条"贴住软键盘"：web 无原生 inputAccessoryView，改用 VisualViewport ——
  // 键盘弹出会缩小可视视口，把整页高度锁到 vv.height、并按 vv.offsetTop 补偿 iOS 的
  // 视口上滚，键条（在页面底部）便浮在键盘正上方；键盘收起时复位。变化后触发终端 refit。
  useEffect(() => {
    const vv = window.visualViewport;
    const el = page.current;
    if (!vv || !el) return;
    let raf = 0;
    const apply = () => {
      el.style.height = vv.height + "px";
      el.style.transform = `translateY(${vv.offsetTop}px)`;
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => window.dispatchEvent(new Event("resize")));
    };
    vv.addEventListener("resize", apply);
    vv.addEventListener("scroll", apply);
    apply();
    return () => {
      vv.removeEventListener("resize", apply);
      vv.removeEventListener("scroll", apply);
      cancelAnimationFrame(raf);
      el.style.height = "";
      el.style.transform = "";
    };
  }, []);

  return (
    <div className="tpage" ref={page}>
      <div className="tbar">
        <div className="l">
          {onBack
            ? <button className="back" onClick={onBack} title="返回"><Icon name="arrowLeft" /></button>
            : <a className="back" href="/" title="返回门户"><Icon name="arrowLeft" /></a>}
          <span className="tile"><Icon name="terminal" /></span>
          <span className="tbrand">AIT.dev</span>
        </div>
        <span className="sp"><span className={"dot" + (conn === true ? " on" : conn === false ? " off" : "")} />{who}</span>
      </div>
      <div className="stage">
        <TermView server={server} ws={ws} onStatus={setConn} />
      </div>
    </div>
  );
}
