"""FastAPI 组装：API/WS 路由 + 托管前端 SPA。

路由顺序要点：具体的 /api、/ws、/vscode 先注册；最后一个 catch-all 只负责
返回前端静态文件或 index.html（SPA 客户端路由回退）。
"""
from contextlib import asynccontextmanager

from fastapi import FastAPI
from fastapi.responses import FileResponse, HTMLResponse

from .config import WEB_DIR
from .routes import (docs_routes, me, password, settings, terminal, upload,
                     vscode_routes, workspaces)


@asynccontextmanager
async def lifespan(app: FastAPI):
    yield
    await vscode_routes.aclose()


def create_app() -> FastAPI:
    app = FastAPI(title="devsys portal", lifespan=lifespan)

    # API
    app.include_router(me.router)
    app.include_router(settings.router)
    app.include_router(password.router)
    app.include_router(workspaces.router)
    app.include_router(upload.router)
    app.include_router(docs_routes.router)
    # web SSH / VS Code
    app.include_router(terminal.router)
    app.include_router(vscode_routes.router)

    @app.get("/healthz")
    def healthz():
        return {"ok": True}

    # ── SPA：其余路径 → 静态文件或 index.html（客户端路由回退）──
    @app.get("/{path:path}", include_in_schema=False)
    def spa(path: str):
        index = WEB_DIR / "index.html"
        if path:
            target = (WEB_DIR / path).resolve()
            if str(target).startswith(str(WEB_DIR.resolve())) and target.is_file():
                return FileResponse(target)
        if index.is_file():
            return FileResponse(index)
        return HTMLResponse("<h1>devsys</h1><p>前端未构建（运行 deploy 或 frontend/ 下 npm run build）。</p>",
                            status_code=200)

    return app


app = create_app()
