// 后端 JSON API 封装 + 类型。

export interface Server {
  name: string;
  host: string;
  port: number;
  jump?: string | null;
  username: string;
  auth: "password" | "key";
  has_secret: boolean;
}
export interface Me {
  user: string;
  email_login: boolean;
  servers: Server[];
}
export interface WsSession {
  name: string;
  created: number;
  windows: number;
  attached: boolean;
  activity: number;
}
export interface WsServer {
  server: string;
  host: string;
  port: number;
  jump?: string | null;
  configured: boolean;
  sessions: WsSession[];
  error: string | null;
}
export type DocNode =
  | { type: "doc"; slug: string; title: string }
  | { type: "group"; title: string; path: string; slug?: string; children: DocNode[] };
export interface Doc {
  slug: string;
  title: string;
  content: string;
}

/** 深度优先找第一篇可打开的文档（组的 _index 也算）。 */
export function firstDocSlug(nodes: DocNode[]): string {
  for (const n of nodes) {
    if (n.type === "doc") return n.slug;
    if (n.slug) return n.slug;
    const c = firstDocSlug(n.children);
    if (c) return c;
  }
  return "";
}

/** 若 slug 是某个分组的着陆页（_index），返回该组的直接子节点；否则返回 []。 */
export function groupChildren(nodes: DocNode[], slug: string): DocNode[] {
  for (const n of nodes) {
    if (n.type !== "group") continue;
    if (n.slug === slug) return n.children;
    const c = groupChildren(n.children, slug);
    if (c.length) return c;
  }
  return [];
}

/** 收集全部可打开的 slug（用于校验记忆的“上次打开”是否仍存在）。 */
export function collectSlugs(nodes: DocNode[], acc: string[] = []): string[] {
  for (const n of nodes) {
    if (n.type === "doc") acc.push(n.slug);
    else { if (n.slug) acc.push(n.slug); collectSlugs(n.children, acc); }
  }
  return acc;
}

export interface Crumb { title: string; slug?: string; }

/** 从根到 slug 的面包屑（含各级分组与当前页；分组无 _index 时 slug 缺省、不可点）。 */
export function breadcrumb(nodes: DocNode[], slug: string): Crumb[] {
  const path: Crumb[] = [];
  const dfs = (list: DocNode[]): boolean => {
    for (const n of list) {
      if (n.type === "doc") {
        if (n.slug === slug) { path.push({ title: n.title, slug: n.slug }); return true; }
      } else {
        if (n.slug === slug) { path.push({ title: n.title, slug: n.slug }); return true; }
        path.push({ title: n.title, slug: n.slug });
        if (dfs(n.children)) return true;
        path.pop();
      }
    }
    return false;
  };
  dfs(nodes);
  return path;
}

export interface PageRef { slug: string; title: string; }

/** 按目录顺序把所有“可打开的页”摊平（分组着陆页排在其子项之前）。 */
export function flatPages(nodes: DocNode[], acc: PageRef[] = []): PageRef[] {
  for (const n of nodes) {
    if (n.type === "doc") acc.push({ slug: n.slug, title: n.title });
    else {
      if (n.slug) acc.push({ slug: n.slug, title: n.title });
      flatPages(n.children, acc);
    }
  }
  return acc;
}

/** 当前页在线性顺序里的上一/下一页。 */
export function neighbors(nodes: DocNode[], slug: string): { prev: PageRef | null; next: PageRef | null } {
  const flat = flatPages(nodes);
  const i = flat.findIndex((p) => p.slug === slug);
  return {
    prev: i > 0 ? flat[i - 1] : null,
    next: i >= 0 && i < flat.length - 1 ? flat[i + 1] : null,
  };
}

async function j<T>(url: string, init?: RequestInit): Promise<T> {
  const r = await fetch(url, init);
  if (!r.ok) throw new Error(String(r.status));
  return r.json() as Promise<T>;
}
function post(body: unknown): RequestInit {
  return { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) };
}

export interface SettingsBody {
  server: string;
  username: string;
  auth: "password" | "key";
  secret?: string;
}

export const api = {
  me: () => j<Me>("/api/me"),
  saveSettings: (b: SettingsBody) => j<{ ok: boolean; has_secret: boolean }>("/api/settings", post(b)),
  changePassword: (password: string) => j<{ ok: boolean }>("/api/password", post({ password })),
  workspaces: () => j<{ servers: WsServer[] }>("/api/workspaces"),
  newWs: (server: string, name: string) => j<{ ok: boolean }>("/api/workspaces/new", post({ server, name })),
  killWs: (server: string, name: string) => j<{ ok: boolean }>("/api/workspaces/kill", post({ server, name })),
  docs: () => j<{ tree: DocNode[] }>("/api/docs"),
  doc: (slug: string) => j<Doc>("/api/docs/" + slug.split("/").map(encodeURIComponent).join("/")),
};
