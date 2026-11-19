#!/usr/bin/env python3
"""Check shared-chat persistence, cancellation, metrics and client concurrency over HTTP."""
import argparse
import contextlib
import fcntl
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
import pty
from pathlib import Path
import signal
import select
import socket
import subprocess
import struct
import tempfile
import termios
import threading
import time
import urllib.error
import urllib.request
import uuid

TOKEN = "workspace-integration-token"


def request(base, path, body=None, token=TOKEN):
    req = urllib.request.Request(base + path, data=None if body is None else json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"})
    try:
        with urllib.request.urlopen(req, timeout=15) as response:
            data = response.read()
            return response.status, json.loads(data) if "application/json" in response.headers.get("Content-Type", "") else data.decode()
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read())


def require(base, path, body=None):
    status, data = request(base, path, body)
    assert 200 <= status < 300, (path, status, data)
    return data


class Runtime(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        data = json.dumps({"data": [{"id": "mock"}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        text = body["messages"][-1]["content"]
        assert "conversation_id" not in body
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        try:
            for index in range(30 if text == "slow" else 2):
                self.wfile.write(("data: " + json.dumps({"choices": [{"delta": {"content": "saved 日本語 "}}],
                      "timings": {"predicted_n": index + 1, "predicted_per_second": 10}}) + "\n\n").encode())
                self.wfile.flush()
                time.sleep(.12)
            if text != "interrupt":
                self.wfile.write(b'data: {"choices":[{"delta":{},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n')
        except (BrokenPipeError, ConnectionResetError):
            pass


def terminal_chat(binary, base, home):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
    def session():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    process = subprocess.Popen([binary,"--connect",base],stdin=slave,stdout=slave,stderr=slave,
        preexec_fn=session,env={**os.environ,"TERM":"xterm-256color","LOCALHOSTAI_TOKEN":TOKEN,"LOCALHOSTAI_HOME":str(home)})
    os.close(slave)
    def pump(seconds):
        output=b""
        deadline=time.monotonic()+seconds
        while time.monotonic()<deadline:
            if select.select([master],[],[],.05)[0]:
                try: output += os.read(master,65536)
                except OSError: break
        return output
    try:
        output=pump(1.5)
        assert b"Ctrl+O" in output,output[-2000:]
        os.write(master,b"\x0e")  # Ctrl+N: preserve previous conversations.
        pump(.8)
        os.write(master,b"terminal note\r")
        pump(1.5)
        chats=require(base,"/conversations")["conversations"]
        terminal=next((c for c in chats if c["title"]=="terminal note"),None)
        assert terminal,chats
        saved=require(base,f'/conversations/{terminal["id"]}')["conversation"]
        assert saved["messages"][0]["content"]=="terminal note" and saved["active_request"] is None,saved
        os.write(master,b"\x0f")  # Ctrl+O: conversation picker.
        pump(.3)
        os.write(master,b"\x05")  # Ctrl+E: Markdown export of current chat.
        pump(1)
        exports=list((home/"exports").glob("*.md"))
        assert len(exports)==1 and "terminal note" in exports[0].read_text()
        print("PASS real terminal creates a saved chat visible to another client and exports Markdown",flush=True)
    finally:
        if process.poll() is None:
            process.terminate()
        process.wait(timeout=5)
        os.close(master)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/localhostai")
    parser.add_argument("--temp-dir", default="/tmp/opencode" if Path("/tmp/opencode").is_dir() else None)
    options = parser.parse_args()
    binary = str(Path(options.binary).resolve())
    with tempfile.TemporaryDirectory(prefix="localhostai-workspace-", dir=options.temp_dir) as directory, contextlib.ExitStack() as stack:
        home = Path(directory)
        runtime = ThreadingHTTPServer(("127.0.0.1", 0), Runtime)
        runtime.daemon_threads = True
        stack.callback(runtime.server_close)
        stack.callback(runtime.shutdown)
        threading.Thread(target=runtime.serve_forever, daemon=True).start()
        (home / "config.toml").write_text(f'engine="llamacpp"\nllamacpp_url="http://127.0.0.1:{runtime.server_port}/v1"\nrequire_pairing=false\ndiscovery=false\n')
        (home / "pair_token").write_text(TOKEN)
        with socket.socket() as port:
            port.bind(("127.0.0.1",0))
            number = port.getsockname()[1]
        base = f"http://127.0.0.1:{number}"
        def stop(process):
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=10)
        def start():
            log = stack.enter_context((home / "node.log").open("a"))
            process = subprocess.Popen([binary,"--headless","--no-lan","--port",str(number)], env={**os.environ,"LOCALHOSTAI_HOME":directory},stdout=log,stderr=log)
            stack.callback(stop,process)
            for _ in range(100):
                try:
                    if require(base,"/health")["ok"]:
                        return process
                    time.sleep(.05)
                except OSError:
                    time.sleep(.05)
            raise AssertionError("coordinator did not start")
        node = start()
        for path in ["/conversations","/performance","/benchmarks"]:
            assert request(base,path,token="wrong")[0] == 401
        first = require(base,"/conversations",{"title":"Shared notes"})["conversation"]
        second = require(base,"/conversations",{"title":"Other notes"})["conversation"]
        def send(chat, text):
            return request(base,"/v1/chat/completions",{"conversation_id":chat["id"],"revision":chat["revision"],"request_id":uuid.uuid4().hex,"content":text,"stream":True,"ctx":2048})
        answer = send(first,"hello")
        assert answer[0] == 200, answer
        loaded = require(base,f'/conversations/{first["id"]}')["conversation"]
        assert len(loaded["messages"]) == 2 and loaded["active_request"] is None
        assert loaded["messages"][1]["content"] == "saved 日本語 saved 日本語 "
        assert request(base,f'/conversations/{first["id"]}',{"revision":first["revision"],"title":"stale"})[0] == 409
        assert require(base,"/conversations?query=%E6%97%A5%E6%9C%AC%E8%AA%9E")["conversations"][0]["id"] == first["id"]
        for fmt in ["json","markdown"]:
            exported = require(base,f'/conversations/{first["id"]}/export?format={fmt}')
            assert "saved" in exported["content"]
        metrics = require(base,"/performance")["generations"][0]
        assert metrics["output_tokens"] == 2 and metrics["ttft_ms"] > 0 and metrics["state"] == "complete"
        print("PASS paired clients share named chats, search/export Unicode, reject stale edits, and observe measured timing",flush=True)
        assert send(second,"interrupt")[0] == 200
        partial = require(base,f'/conversations/{second["id"]}')["conversation"]
        assert partial["messages"][-1]["interrupted"] and partial["messages"][-1]["content"]
        thread = threading.Thread(target=send,args=(partial,"slow"),daemon=True)
        thread.start()
        for _ in range(80):
            active = require(base,f'/conversations/{second["id"]}')["conversation"]
            if active["active_request"] and active["messages"][-1]["content"]:
                break
            time.sleep(.05)
        assert active["active_request"]
        require(base,f'/conversations/{second["id"]}/cancel',{"request_id":active["active_request"]})
        thread.join(timeout=5)
        assert not thread.is_alive()
        after = require(base,f'/conversations/{second["id"]}')["conversation"]
        assert after["active_request"] is None and after["messages"][-1]["interrupted"]
        print("PASS interrupted text persists and another client can cancel the shared generation",flush=True)
        terminal_chat(binary,base,home)
        before = require(base,"/conversations")
        stop(node)
        node = start()
        restored = require(base,"/conversations")
        assert restored == before
        assert require(base,f'/conversations/{first["id"]}')["conversation"] == loaded
        assert (home / "conversations" / f'{first["id"]}.json').stat().st_mode & 0o077 == 0
        print("PASS coordinator restart restores conversations with private file permissions",flush=True)
        require(base,f'/conversations/{first["id"]}/delete',{"revision":loaded["revision"]})
        assert request(base,f'/conversations/{first["id"]}')[0] == 404
        print("PASS deletion is shared across clients",flush=True)


if __name__ == "__main__":
    main()
