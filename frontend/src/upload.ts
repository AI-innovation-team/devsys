// 高速上传：把文件/目录在浏览器端打成 tar → 原生 gzip → 分块可续传地 PUT 到门户。
// 零第三方依赖。tar 用 GNU 长名扩展支持任意长度路径；gzip 用原生 CompressionStream。
// 见后端 routes/upload.py 与文档「功能详解 › 高速上传」。

export interface UploadFile { path: string; file: File; }

export interface ResolvedPath { path: string; exists: boolean; isdir: boolean; error?: string; }

/** 向目标机询问某路径的绝对形式与存在性（用于显示完整路径 + 实时校验）。 */
export async function resolvePath(server: string, path: string): Promise<ResolvedPath> {
  const q = `?server=${encodeURIComponent(server)}&path=${encodeURIComponent(path)}`;
  const r = await fetch("/api/upload/resolve" + q);
  if (!r.ok) throw new Error("resolve " + r.status);
  return (await r.json()) as ResolvedPath;
}

const enc = new TextEncoder();

// ── 拖拽/选择 → 文件清单（带相对路径，用于在 tar 里重建目录结构）──

export async function filesFromDrop(dt: DataTransfer): Promise<UploadFile[]> {
  const entries = Array.from(dt.items)
    .map((i) => (i.webkitGetAsEntry ? i.webkitGetAsEntry() : null))
    .filter(Boolean) as FileSystemEntry[];
  const out: UploadFile[] = [];
  if (entries.length) {
    for (const e of entries) await walkEntry(e, "", out);
  } else {
    for (const f of Array.from(dt.files)) out.push({ path: f.name, file: f });
  }
  return out;
}

export function filesFromInput(list: FileList): UploadFile[] {
  return Array.from(list).map((f) => ({
    path: (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name,
    file: f,
  }));
}

async function walkEntry(entry: FileSystemEntry, prefix: string, out: UploadFile[]): Promise<void> {
  if (entry.isFile) {
    const file = await new Promise<File>((res, rej) => (entry as FileSystemFileEntry).file(res, rej));
    out.push({ path: prefix + entry.name, file });
  } else if (entry.isDirectory) {
    const rd = (entry as FileSystemDirectoryEntry).createReader();
    // readEntries 每次最多返回一批，需反复读到空为止
    let batch: FileSystemEntry[];
    do {
      batch = await new Promise<FileSystemEntry[]>((res, rej) => rd.readEntries(res, rej));
      for (const e of batch) await walkEntry(e, prefix + entry.name + "/", out);
    } while (batch.length);
  }
}

// ── tar 编码（USTAR + GNU 长名）──

function octal(buf: Uint8Array, off: number, len: number, val: number) {
  const s = Math.floor(val).toString(8).padStart(len - 1, "0").slice(-(len - 1));
  buf.set(enc.encode(s), off);
  buf[off + len - 1] = 0;
}

function header(name: string, size: number, type: "0" | "L", mtime: number): Uint8Array {
  const buf = new Uint8Array(512);
  buf.set(enc.encode(name).slice(0, 100), 0);
  octal(buf, 100, 8, 0o644);         // mode
  octal(buf, 108, 8, 0);             // uid
  octal(buf, 116, 8, 0);             // gid
  octal(buf, 124, 12, size);         // size
  octal(buf, 136, 12, mtime);        // mtime
  for (let i = 148; i < 156; i++) buf[i] = 0x20; // chksum 先填空格
  buf[156] = type.charCodeAt(0);     // typeflag
  buf.set(enc.encode("ustar"), 257); // magic
  buf[263] = 0x30; buf[264] = 0x30;  // version "00"
  let sum = 0;
  for (let i = 0; i < 512; i++) sum += buf[i];
  buf.set(enc.encode(sum.toString(8).padStart(6, "0")), 148);
  buf[154] = 0; buf[155] = 0x20;
  return buf;
}

/** 产出 tar 字节流；边读文件边 yield，内存占用与单块无关。onRead 汇报已读(未压缩)字节用于进度。 */
async function* tarBytes(files: UploadFile[], onRead: (n: number) => void): AsyncGenerator<Uint8Array> {
  for (const { path, file } of files) {
    const nb = enc.encode(path);
    if (nb.length > 100) {
      // GNU 长名：前置一块 typeflag 'L'，内容为完整路径
      yield header("././@LongLink", nb.length + 1, "L", 0);
      const body = new Uint8Array(Math.ceil((nb.length + 1) / 512) * 512);
      body.set(nb, 0);
      yield body;
    }
    yield header(path, file.size, "0", Math.floor(file.lastModified / 1000) || 0);
    const reader = file.stream().getReader();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      onRead(value.length);
      yield value;
    }
    const pad = (512 - (file.size % 512)) % 512;
    if (pad) yield new Uint8Array(pad);
  }
  yield new Uint8Array(1024); // 结尾两个零块
}

function iterToStream(it: AsyncGenerator<Uint8Array>): ReadableStream<Uint8Array> {
  return new ReadableStream<Uint8Array>({
    async pull(ctrl) {
      const { done, value } = await it.next();
      if (done) ctrl.close();
      else ctrl.enqueue(value);
    },
  });
}

function merge(list: Uint8Array[]): Uint8Array {
  const total = list.reduce((n, a) => n + a.length, 0);
  const out = new Uint8Array(total);
  let o = 0;
  for (const a of list) { out.set(a, o); o += a.length; }
  return out;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// ── 分块可续传上传 ──

export type UploadPhase = "packing" | "sending" | "finishing" | "done" | "error" | "canceled";
export interface UploadState {
  phase: UploadPhase;
  sent: number;            // 已发送(压缩后)字节
  read: number;            // 已读(未压缩)字节
  total: number;           // 未压缩总字节
  rate: number;            // 压缩后发送速率 B/s
  eta: number;             // 预计剩余秒
  error?: string;
}
export interface UploadHandle { promise: Promise<void>; cancel: () => void; }

const CHUNK = 8 * 1024 * 1024;

/** 起一个上传任务。server/dest 为目标机与目标目录；files 为清单；onState 汇报进度。 */
export function startUpload(
  server: string,
  dest: string,
  files: UploadFile[],
  onState: (s: UploadState) => void,
): UploadHandle {
  const ac = new AbortController();
  const total = files.reduce((n, f) => n + f.file.size, 0);
  const started = Date.now();
  const st: UploadState = { phase: "packing", sent: 0, read: 0, total, rate: 0, eta: 0 };
  const tick = () => {
    const el = (Date.now() - started) / 1000;
    st.rate = el > 0 ? st.sent / el : 0;
    // ETA 用未压缩读取速率估算（读取节奏≈整体进度）
    const rrate = el > 0 ? st.read / el : 0;
    st.eta = rrate > 0 ? Math.max(0, (total - st.read) / rrate) : 0;
    onState({ ...st });
  };

  const run = async () => {
    if (typeof CompressionStream === "undefined") {
      throw new Error("浏览器不支持 CompressionStream（请用较新版 Chrome/Edge/Safari）");
    }
    const label = files.length === 1 ? files[0].path : `${files.length} 个文件`;
    const initRes = await fetch("/api/upload/init", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ server, dest, filename: label, total }),
      signal: ac.signal,
    });
    if (!initRes.ok) throw new Error("初始化失败 " + initRes.status);
    const { id } = (await initRes.json()) as { id: string };

    let offset = 0;
    const putBlock = async (block: Uint8Array) => {
      for (let attempt = 0; ; attempt++) {
        if (ac.signal.aborted) throw new DOMException("aborted", "AbortError");
        try {
          const r = await fetch(`/api/upload/${id}?offset=${offset}`, { method: "PUT", body: block, signal: ac.signal });
          if (r.status === 409) {
            // 丢包幂等：上一块其实已落盘、只是响应丢了 → 服务端已收 offset+len，视为成功
            const j = await r.json().catch(() => ({}));
            const rec = j?.detail?.received ?? j?.received;
            if (rec === offset + block.length) { offset = rec; st.sent += block.length; tick(); return; }
            throw new Error("偏移失步");
          }
          if (!r.ok) throw new Error("HTTP " + r.status);
          const j = (await r.json()) as { received: number };
          offset = j.received; st.sent += block.length; tick();
          return;
        } catch (e) {
          if (ac.signal.aborted) throw e;
          if (attempt >= 6) throw e;           // 约 1+2+4+8+15+15s 的退避后放弃
          await sleep(Math.min(1000 * 2 ** attempt, 15000));
        }
      }
    };

    st.phase = "sending"; tick();
    const gz = iterToStream(tarBytes(files, (n) => { st.read += n; }))
      .pipeThrough(new CompressionStream("gzip"));
    const reader = gz.getReader();
    let pending: Uint8Array[] = [];
    let pendLen = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (value && value.length) { pending.push(value); pendLen += value.length; }
      if (pendLen >= CHUNK) {
        const all = merge(pending);
        let o = 0;
        while (all.length - o >= CHUNK) { await putBlock(all.subarray(o, o + CHUNK)); o += CHUNK; }
        const rem = all.subarray(o);
        pending = rem.length ? [rem.slice()] : [];
        pendLen = rem.length;
      }
      if (done) {
        if (pendLen > 0) await putBlock(merge(pending));
        break;
      }
    }

    st.phase = "finishing"; tick();
    const fin = await fetch(`/api/upload/${id}/finish`, { method: "POST", signal: ac.signal });
    if (!fin.ok) {
      let msg = "解包失败 " + fin.status;
      try { const j = await fin.json(); if (j?.detail) msg = typeof j.detail === "string" ? j.detail : msg; } catch { /* ignore */ }
      throw new Error(msg);
    }
    st.phase = "done"; tick();
  };

  const promise = run().catch((e) => {
    if (ac.signal.aborted) { st.phase = "canceled"; tick(); return; }
    st.phase = "error"; st.error = e instanceof Error ? e.message : String(e); tick();
    throw e;
  });

  return { promise, cancel: () => ac.abort() };
}
