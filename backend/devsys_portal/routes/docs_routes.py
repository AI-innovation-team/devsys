"""文档：GET /api/docs（层级树）、GET /api/docs/{slug:path}（原文）。

slug 支持斜杠（层级路径），按白名单匹配真实文件，杜绝路径穿越。
"""
from fastapi import APIRouter, Depends, HTTPException

from ..auth import current_user
from ..docs import doc_path, doc_tree, doc_view

router = APIRouter()


@router.get("/api/docs")
def list_docs(user: str = Depends(current_user)):
    return {"tree": doc_tree()}


@router.get("/api/docs/{slug:path}")
def get_doc(slug: str, user: str = Depends(current_user)):
    p = doc_path(slug)
    if not p:
        raise HTTPException(404, "文档不存在")
    title, content = doc_view(p)
    return {"slug": slug, "title": title, "content": content}
