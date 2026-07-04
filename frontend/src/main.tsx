import { createRoot } from "react-dom/client";

import { App } from "./App";
import { Terminal } from "./screens/Terminal";
import "./styles/tokens.css";
import "./styles/app.css";

// 极简路由：/terminal/<server> 独立整页终端（新标签打开）；其余走门户 SPA。
const root = createRoot(document.getElementById("root")!);
const path = location.pathname;

if (path.startsWith("/terminal/")) {
  const server = decodeURIComponent(path.slice("/terminal/".length).split("/")[0]);
  const ws = new URLSearchParams(location.search).get("ws") || "";
  root.render(<Terminal server={server} ws={ws} />);
} else {
  root.render(<App />);
}
