"""文档：GET /api/docs（列表）、GET /api/docs/{slug}（原文）。

按真实文件名白名单匹配 slug，杜绝路径穿越。
"""
from fastapi import APIRouter, Depends, HTTPException

from ..auth import current_user
from ..docs import doc_files, doc_title

router = APIRouter()


@router.get("/api/docs")
def list_docs(user: str = Depends(current_user)):
    return {"docs": [{"slug": p.stem, "title": doc_title(p)} for p in doc_files()]}


@router.get("/api/docs/{slug}")
def get_doc(slug: str, user: str = Depends(current_user)):
    p = next((f for f in doc_files() if f.stem == slug), None)
    if not p:
        raise HTTPException(404, "文档不存在")
    return {"slug": p.stem, "title": doc_title(p), "content": p.read_text(encoding="utf-8")}
