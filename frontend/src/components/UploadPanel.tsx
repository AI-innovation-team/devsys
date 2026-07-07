import { DragEvent as ReactDragEvent, useEffect, useRef, useState } from "react";

import { Icon } from "../icons";
import { filesFromDrop, filesFromInput, resolvePath, ResolvedPath, startUpload, UploadFile, UploadHandle, UploadState } from "../upload";

interface Job { key: string; label: string; state: UploadState; handle: UploadHandle; }

let seq = 0;

function human(n: number): string {
  if (n < 1024) return n + " B";
  if (n < 1024 * 1024) return (n / 1024).toFixed(1) + " KB";
  if (n < 1024 * 1024 * 1024) return (n / 1024 / 1024).toFixed(1) + " MB";
  return (n / 1024 / 1024 / 1024).toFixed(2) + " GB";
}
function fmtEta(s: number): string {
  s = Math.round(s);
  if (s < 60) return s + " 秒";
  if (s < 3600) return Math.floor(s / 60) + " 分" + (s % 60) + " 秒";
  return Math.floor(s / 3600) + " 时" + Math.floor((s % 3600) / 60) + " 分";
}

export function UploadPanel({ servers, defaultServer }: { servers: string[]; defaultServer?: string }) {
  const [server, setServer] = useState(defaultServer || servers[0] || "");
  const [dest, setDest] = useState("");
  const [check, setCheck] = useState<ResolvedPath | null>(null);
  const [checking, setChecking] = useState(false);
  const [drag, setDrag] = useState(false);
  const [jobs, setJobs] = useState<Job[]>([]);
  const fileRef = useRef<HTMLInputElement>(null);
  const dirRef = useRef<HTMLInputElement>(null);
  const homeTok = useRef(0);
  const chkTok = useRef(0);

  // 选服务器 → 向目标机要家目录的绝对路径填入（显示完整路径，不再是 ~）
  useEffect(() => {
    if (!server) return;
    const tok = ++homeTok.current;
    setCheck(null);
    resolvePath(server, "")
      .then((r) => { if (tok === homeTok.current) setDest(r.path || "~"); })
      .catch(() => { if (tok === homeTok.current) setDest("~"); });
  }, [server]);

  // 改路径（含上面自动填入）→ 防抖核验该路径在目标机上是否真实存在
  useEffect(() => {
    if (!server || !dest) { setCheck(null); setChecking(false); return; }
    setChecking(true);
    const tok = ++chkTok.current;
    const t = setTimeout(() => {
      resolvePath(server, dest)
        .then((r) => { if (tok === chkTok.current) { setCheck(r); setChecking(false); } })
        .catch(() => { if (tok === chkTok.current) { setCheck(null); setChecking(false); } });
    }, 400);
    return () => clearTimeout(t);
  }, [server, dest]);

  // 目标已存在但不是目录 → 不能解包到此，拦截
  const pathBad = !!check && check.exists && !check.isdir;

  const begin = (files: UploadFile[]) => {
    if (!files.length || !server || pathBad) return;
    const key = "u" + ++seq;
    const label = files.length === 1 ? files[0].path : `${files.length} 个文件`;
    const total = files.reduce((n, f) => n + f.file.size, 0);
    const handle = startUpload(server, dest, files, (state) => {
      setJobs((js) => js.map((j) => (j.key === key ? { ...j, state } : j)));
    });
    setJobs((js) => [{ key, label, state: { phase: "packing", sent: 0, read: 0, total, rate: 0, eta: 0 }, handle }, ...js]);
    handle.promise.catch(() => { /* 状态已在 onState 里反映 */ });
  };

  const onDrop = async (e: ReactDragEvent) => {
    e.preventDefault();
    setDrag(false);
    begin(await filesFromDrop(e.dataTransfer));
  };

  return (
    <div className="up-panel">
      <div className="up-cfg">
        <div className="inp sel">
          <Icon name="server" />
          <select value={server} onChange={(e) => setServer(e.target.value)}>
            {servers.map((s) => <option key={s} value={s}>{s}</option>)}
          </select>
        </div>
        <div className={"inp" + (pathBad ? " bad" : "")}>
          <Icon name="folder" />
          <input value={dest} onChange={(e) => setDest(e.target.value)} placeholder="解析中…" spellCheck={false} />
        </div>
      </div>

      <div className={"up-check" + (pathBad ? " bad" : check && !check.exists ? " new" : "")}>
        {checking ? (
          <span>核验路径中…</span>
        ) : check?.error ? (
          <><Icon name="alert" />无法访问目标机：{check.error}</>
        ) : pathBad ? (
          <><Icon name="alert" />目标已存在且不是目录，无法上传到此路径</>
        ) : check && check.exists ? (
          <><Icon name="check" />目录已存在 · <code>{check.path}</code></>
        ) : check ? (
          <><Icon name="folder" />目录不存在，上传时将自动创建 · <code>{check.path}</code></>
        ) : null}
      </div>

      <div
        className={"up-drop" + (drag ? " over" : "") + (pathBad ? " disabled" : "")}
        onDragOver={(e) => { if (!pathBad) { e.preventDefault(); setDrag(true); } }}
        onDragLeave={() => setDrag(false)}
        onDrop={onDrop}
      >
        <Icon name="upload" className="up-drop-ic" />
        <div className="up-drop-t">拖拽文件或文件夹到此处</div>
        <div className="up-drop-sub">gzip 压缩 + 分块续传，直达 <b>{server || "目标机"}</b></div>
        <div className="up-drop-btns">
          <button className="btn secondary sm" disabled={pathBad} onClick={() => fileRef.current?.click()}><Icon name="file" />选文件</button>
          <button className="btn secondary sm" disabled={pathBad} onClick={() => dirRef.current?.click()}><Icon name="folder" />选文件夹</button>
        </div>
        <input ref={fileRef} type="file" multiple hidden
          onChange={(e) => { if (e.target.files) begin(filesFromInput(e.target.files)); e.target.value = ""; }} />
        <input ref={dirRef} type="file" hidden
          onChange={(e) => { if (e.target.files) begin(filesFromInput(e.target.files)); e.target.value = ""; }}
          {...({ webkitdirectory: "", directory: "" } as Record<string, string>)} />
      </div>

      {jobs.length > 0 && (
        <div className="up-jobs">
          {jobs.map((j) => <JobRow key={j.key} job={j} onClose={() => setJobs((js) => js.filter((x) => x.key !== j.key))} />)}
        </div>
      )}
    </div>
  );
}

function JobRow({ job, onClose }: { job: Job; onClose: () => void }) {
  const s = job.state;
  const active = s.phase === "packing" || s.phase === "sending" || s.phase === "finishing";
  const pct = s.phase === "done" ? 100 : s.total > 0 ? Math.min(99, Math.floor((s.read / s.total) * 100)) : 0;
  const status: Record<string, string> = {
    packing: "准备中…",
    sending: `发送中 · ${human(s.rate)}/s`,
    finishing: "目标机解包中…",
    done: "完成",
    error: s.error || "失败",
    canceled: "已取消",
  };
  const ic = s.phase === "done" ? "check" : s.phase === "error" ? "alert" : s.phase === "canceled" ? "x" : "upload";
  return (
    <div className={"up-job " + s.phase}>
      <div className="up-job-head">
        <Icon name={ic} className="up-job-ic" />
        <span className="up-job-name" title={job.label}>{job.label}</span>
        <span className="up-job-status">{status[s.phase]}</span>
        <button className="btn subtle sm" title={active ? "取消" : "移除"} onClick={() => (active ? job.handle.cancel() : onClose())}>
          <Icon name="x" />
        </button>
      </div>
      <div className="up-bar"><div className="up-bar-fill" style={{ width: pct + "%" }} /></div>
      <div className="up-job-meta">
        <span>{human(s.sent)} 已发送{s.total ? ` · 原始 ${human(s.total)}` : ""}</span>
        {s.phase === "sending" && s.eta > 0 && <span>剩约 {fmtEta(s.eta)}</span>}
      </div>
    </div>
  );
}
