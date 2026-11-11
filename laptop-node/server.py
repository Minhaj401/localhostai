"""
PrivateLM-LAN laptop node.

Runs on your laptop and exposes an OpenAI-compatible chat API backed by
whatever inference engine is already installed locally (Ollama or
llama.cpp's llama-server). Also answers UDP discovery broadcasts so the
mobile app can find it automatically.

Endpoints:
  GET  /health                 -> {"ok": true, "name": ..., "backend": ..., "models": [...]}
  GET  /v1/models              -> OpenAI-style model list (proxied)
  POST /v1/chat/completions    -> OpenAI-style chat, streaming + non-streaming

Backends (env PRIVATELM_BACKEND, default: auto):
  ollama    -> proxy to http://127.0.0.1:11434/v1
  llamacpp  -> proxy to http://127.0.0.1:8080/v1  (llama-server)
  auto      -> probe Ollama first, then llama-server

Discovery:
  Listens for UDP broadcast "PRIVATELM_DISCOVER" on port 50897 and replies
  with a one-line JSON descriptor (name, port, backend, models).
"""

import asyncio
import contextlib
import json
import os
import socket
import subprocess
import threading
from typing import AsyncIterator

import httpx
import uvicorn
from fastapi import FastAPI, HTTPException, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import StreamingResponse
from fastapi.staticfiles import StaticFiles

DISCOVERY_PORT = 50897
DISCOVERY_MAGIC = "PRIVATELM_DISCOVER"
RPC_PORT = 50052          # llama.cpp rpc-server default
LLAMA_SERVER_PORT = 8081  # distributed llama-server (Ollama owns 11434)

NODE_NAME = os.environ.get("PRIVATELM_NODE_NAME", socket.gethostname())
OLLAMA_URL = os.environ.get("PRIVATELM_OLLAMA_URL", "http://127.0.0.1:11434/v1").rstrip("/")
LLAMACPP_URL = os.environ.get("PRIVATELM_LLAMACPP_URL", "http://127.0.0.1:8080/v1").rstrip("/")
PORT = int(os.environ.get("PRIVATELM_PORT", "8765"))

app = FastAPI(title="PrivateLM-LAN node")
# Browser-based clients (Flutter web) need CORS to call the node.
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_methods=["*"],
    allow_headers=["*"],
)
client = httpx.AsyncClient(timeout=httpx.Timeout(600.0, connect=10.0))
_state = {"backend": None, "base_url": None, "models_cache": []}
_shutdown = threading.Event()
_refresh_task = None
_cluster_lock = asyncio.Lock()

# Optional: serve the Flutter web build at /app/ (off by default).
# Enable with PRIVATELM_SERVE_WEB=1 if you want the browser client.
_WEB_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "web-build")
if os.environ.get("PRIVATELM_SERVE_WEB") == "1" and os.path.isdir(_WEB_DIR):
    app.mount("/app", StaticFiles(directory=_WEB_DIR, html=True), name="app")
    print(f"[node] serving web app at /app/ from {_WEB_DIR}")


async def _probe(url: str) -> bool:
    """True if <url>/models answers like an OpenAI-compatible server."""
    try:
        r = await client.get(f"{url.rstrip('/')}/models", timeout=2.0)
        return r.status_code == 200 and isinstance(r.json().get("data"), list)
    except Exception:
        return False


async def _pick_backend() -> tuple[str, str]:
    requested = os.environ.get("PRIVATELM_BACKEND", "auto")
    if requested not in ("auto", "ollama", "llamacpp"):
        raise RuntimeError(f"unknown backend '{requested}'")
    if requested != "auto":
        url = OLLAMA_URL if requested == "ollama" else LLAMACPP_URL
        if not await _probe(url):
            raise RuntimeError(f"backend '{requested}' not reachable at {url}")
        return requested, url
    for name, url in (("ollama", OLLAMA_URL), ("llamacpp", LLAMACPP_URL)):
        if await _probe(url):
            return name, url
    raise RuntimeError(
        "no local backend found — start Ollama (ollama serve) or llama-server"
    )


async def _list_models() -> list[str]:
    try:
        if not _state["base_url"]:
            return []
        r = await client.get(f"{_state['base_url'].rstrip('/')}/models", timeout=2.0)
        r.raise_for_status()
        return [m["id"] for m in r.json().get("data", [])]
    except Exception:
        return []


@app.on_event("startup")
async def startup() -> None:
    global _refresh_task
    _shutdown.clear()
    try:
        name, url = await _pick_backend()
        _state["backend"], _state["base_url"] = name, url
        print(f"[node] backend: {name} at {url}")
    except RuntimeError as e:
        # Stay up so the app can show a clear error instead of connection-refused.
        _state["backend"], _state["base_url"] = "none", None
        print(f"[node] WARNING: {e}")
    _state["models_cache"] = await _list_models()
    threading.Thread(target=_discovery_responder, daemon=True).start()
    _refresh_task = asyncio.create_task(_refresh_backends())


async def _refresh_backends() -> None:
    while True:
        await asyncio.sleep(15)
        proc = _cluster["proc"]
        if proc is not None and proc.poll() is None:
            continue
        try:
            name, url = await _pick_backend()
            if _cluster["proc"] is not proc:
                continue
            _state["backend"], _state["base_url"] = name, url
        except RuntimeError:
            if _cluster["proc"] is not proc:
                continue
            _state["backend"], _state["base_url"] = "none", None
        _state["models_cache"] = await _list_models()


@app.on_event("shutdown")
async def shutdown() -> None:
    _shutdown.set()
    if _refresh_task is not None:
        _refresh_task.cancel()
        with contextlib.suppress(asyncio.CancelledError):
            await _refresh_task
    async with _cluster_lock:
        await _stop_cluster_process()
    await client.aclose()


@app.get("/health")
async def health() -> dict:
    if _state["base_url"]:
        _state["models_cache"] = await _list_models()
    return {
        "ok": _state["backend"] not in (None, "none"),
        "name": NODE_NAME,
        "backend": _state["backend"],
        "models": _state["models_cache"],
    }


@app.get("/v1/models")
async def models() -> dict:
    return {"object": "list", "data": [{"id": m, "object": "model"} for m in await _list_models()]}


@app.post("/v1/chat/completions")
async def chat_completions(request: Request):
    if not _state["base_url"]:
        raise HTTPException(503, "no inference backend available on this node")
    try:
        body = await request.json()
    except ValueError as e:
        raise HTTPException(400, "invalid JSON") from e
    if not isinstance(body, dict):
        raise HTTPException(400, "request must be a JSON object")
    if not body.get("model"):
        available = _state["models_cache"] or await _list_models()
        if not available:
            raise HTTPException(503, "no model is available on this node")
        body["model"] = available[0]
    stream = bool(body.get("stream", False))
    headers = {"Authorization": request.headers.get("Authorization", "")}
    headers = {k: v for k, v in headers.items() if v}

    if not stream:
        r = await client.post(f"{_state['base_url']}/chat/completions", json=body, headers=headers)
        if r.status_code >= 400:
            raise HTTPException(r.status_code, r.text)
        return r.json()

    async def relay() -> AsyncIterator[bytes]:
        async with client.stream(
            "POST", f"{_state['base_url']}/chat/completions", json=body, headers=headers
        ) as upstream:
            if upstream.status_code >= 400:
                detail = (await upstream.aread()).decode(errors="replace")
                yield f"data: {json.dumps({'error': detail})}\n\n".encode()
                return
            async for line in upstream.aiter_lines():
                if line.startswith("data:"):
                    yield f"{line}\n\n".encode()
                    if line.strip() == "data: [DONE]":
                        return

    return StreamingResponse(relay(), media_type="text/event-stream")


def _discovery_responder() -> None:
    """Reply to UDP broadcasts so phones can find this node without typing IPs."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        sock.bind(("", DISCOVERY_PORT))
    except OSError as e:
        print(f"[node] discovery disabled (port {DISCOVERY_PORT}: {e})")
        sock.close()
        return
    print(f"[node] discovery listening on UDP :{DISCOVERY_PORT}")
    sock.settimeout(0.5)
    while not _shutdown.is_set():
        try:
            data, addr = sock.recvfrom(256)
            if data.decode(errors="ignore").strip() != DISCOVERY_MAGIC:
                continue
            reply = json.dumps({
                "name": NODE_NAME,
                "port": PORT,
                "backend": _state["backend"],
                "models": _state["models_cache"],
            })
            sock.sendto(reply.encode(), addr)
        except socket.timeout:
            continue
        except Exception as e:
            print(f"[node] discovery error: {e}")
    sock.close()


# ── Distributed inference (llama.cpp RPC) ─────────────────────────────
# Phones (or other machines) run `rpc-server`; this node launches
# llama-server with --rpc <peers> so model layers + KV cache are split
# across every device's memory. Borrowed from PrivateLM's philosophy:
# check RAM before loading, scale threads to the device tier.

_cluster = {"proc": None, "peers": [], "model": None}


def _local_ip() -> str:
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(("8.8.8.8", 80))  # no packet sent; picks the LAN source IP
        return s.getsockname()[0]
    except Exception:
        return "127.0.0.1"
    finally:
        s.close()


def _subnet_prefix() -> str:
    return ".".join(_local_ip().split(".")[:3])


async def _scan_rpc_peers() -> list[str]:
    """TCP-probe every host in our /24 on the llama.cpp RPC port."""
    prefix = _subnet_prefix()
    me = _local_ip()

    async def probe(ip: str) -> str | None:
        try:
            _, w = await asyncio.wait_for(
                asyncio.open_connection(ip, RPC_PORT), timeout=0.5)
            w.close()
            await w.wait_closed()
            return f"{ip}:{RPC_PORT}"
        except Exception:
            return None

    tasks = [probe(f"{prefix}.{i}") for i in range(1, 255) if f"{prefix}.{i}" != me]
    found = await asyncio.gather(*tasks)
    return [p for p in found if p]


def _llama_server_bin() -> str:
    return os.environ.get(
        "PRIVATELM_LLAMA_SERVER",
        os.path.expanduser("~/tools/llama.cpp/build/bin/llama-server"))


def _cluster_command(model: str, peers: list[str], ctx: int, threads: int, extra: list) -> list[str]:
    cmd = [
        _llama_server_bin(), "-m", model,
        "--host", "127.0.0.1", "--port", str(LLAMA_SERVER_PORT),
        "-c", str(ctx), "-t", str(threads), "-ngl", "999" if peers else "0",
    ]
    if peers:
        cmd += ["--rpc", ",".join(peers), "--split-mode", "layer"]
    return cmd + [str(arg) for arg in extra]


async def _stop_cluster_process() -> None:
    proc = _cluster["proc"]
    _cluster.update(proc=None, peers=[], model=None)
    if proc is not None:
        if proc.poll() is None:
            proc.terminate()
        try:
            await asyncio.to_thread(proc.wait, timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            await asyncio.to_thread(proc.wait)
    if _state["backend"] == "distributed":
        _state.update(backend="none", base_url=None, models_cache=[])


@app.get("/cluster")
async def cluster_status() -> dict:
    peers = await _scan_rpc_peers()
    running = _cluster["proc"] is not None and _cluster["proc"].poll() is None
    return {
        "rpc_peers": peers,
        "running": running,
        "model": _cluster["model"],
        "endpoint": f"http://127.0.0.1:{LLAMA_SERVER_PORT}/v1" if running else None,
        "llama_server": _llama_server_bin(),
        "llama_server_present": os.path.exists(_llama_server_bin()),
    }


@app.post("/cluster/start")
async def cluster_start(request: Request) -> dict:
    """body: {"model": "/path/gguf", "peers": ["ip:50052", ...], "ctx": 4096}
    Peers optional — the whole /24 is scanned when omitted."""
    try:
        body = await request.json()
    except ValueError as e:
        raise HTTPException(400, "invalid JSON") from e
    if not isinstance(body, dict):
        raise HTTPException(400, "request must be a JSON object")
    model = body.get("model") or os.environ.get("PRIVATELM_CLUSTER_MODEL")
    if not model or not os.path.isfile(model):
        raise HTTPException(400, f"model file not found: {model}")
    if not os.path.exists(_llama_server_bin()):
        raise HTTPException(400,
            "llama-server not built — run scripts/setup-distributed.sh first")

    peers = body.get("peers")
    if peers is None:
        peers = await _scan_rpc_peers()
    if not isinstance(peers, list) or not all(isinstance(p, str) and p for p in peers):
        raise HTTPException(400, "peers must be a list of RPC addresses")
    try:
        ctx = int(body.get("ctx", 4096))
    except (ValueError, TypeError) as e:
        raise HTTPException(400, "ctx must be a positive integer") from e
    if ctx <= 0:
        raise HTTPException(400, "ctx must be a positive integer")
    threads = min(6, os.cpu_count() or 4)
    extra = body.get("extra_args") or []
    if not isinstance(extra, list):
        raise HTTPException(400, "extra_args must be a list")
    cmd = _cluster_command(model, peers, ctx, threads, extra)
    async with _cluster_lock:
        await _stop_cluster_process()
        with open("/tmp/privatelm-cluster.log", "w") as log:
            proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT)
        _cluster.update(proc=proc, peers=peers, model=model)

    # wait for the server to come up
    for _ in range(60):
        await asyncio.sleep(1)
        if _cluster["proc"] is not proc:
            raise HTTPException(409, "cluster start cancelled")
        if proc.poll() is not None:
            async with _cluster_lock:
                if _cluster["proc"] is proc:
                    await _stop_cluster_process()
            raise HTTPException(500, "llama-server exited — see /tmp/privatelm-cluster.log")
        try:
            r = await client.get(f"http://127.0.0.1:{LLAMA_SERVER_PORT}/health", timeout=1.0)
            if r.status_code == 200 and _cluster["proc"] is proc and proc.poll() is None:
                # switch this node's backend to the distributed server
                _state["backend"] = "distributed"
                _state["base_url"] = f"http://127.0.0.1:{LLAMA_SERVER_PORT}/v1"
                _state["models_cache"] = await _list_models()
                return {"ok": True, "peers": peers, "model": model,
                        "endpoint": _state["base_url"]}
        except Exception:
            pass
    async with _cluster_lock:
        if _cluster["proc"] is proc:
            await _stop_cluster_process()
    raise HTTPException(504, "llama-server did not become healthy in 60s")


@app.post("/cluster/stop")
async def cluster_stop() -> dict:
    async with _cluster_lock:
        await _stop_cluster_process()
    # fall back to whatever local backend exists
    try:
        name, url = await _pick_backend()
        _state["backend"], _state["base_url"] = name, url
        _state["models_cache"] = await _list_models()
    except RuntimeError:
        _state["backend"], _state["base_url"] = "none", None
        _state["models_cache"] = []
    return {"ok": True, "backend": _state["backend"]}


if __name__ == "__main__":
    uvicorn.run(app, host="0.0.0.0", port=PORT, log_level="warning")
