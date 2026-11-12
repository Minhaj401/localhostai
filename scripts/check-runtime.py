#!/usr/bin/env python3
"""Exercise the compiled CLI against an isolated local mock node.

Usage: python3 scripts/check-runtime.py [--binary target/debug/localhostai]
Optional: --llama-server PATH --rpc-server PATH checks native RPC device flags.
All child processes and temporary configuration belong to this check.
"""
import argparse
import contextlib
import fcntl
import http.server
import json
import os
from pathlib import Path
import pty
import queue
import select
import socket
import struct
import subprocess
import tempfile
import termios
import threading
import time


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, code, value):
        body = json.dumps(value).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health":
            self.reply(200, {"ok": True, "name": "mock", "backend": "test", "models": ["test-model"], "pairing": True})
        elif self.path == "/v1/models":
            self.reply(200, {"data": [{"id": "test-model"}]})
        elif self.path == "/peers":
            self.reply(200, {"devices": []})
        elif self.path == "/models":
            self.reply(200, {"models": []})
        else:
            self.reply(200, {"state": "stopped"})

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
        if self.server.require_token and self.headers.get("Authorization") != "Bearer MixedCaseToken":
            self.reply(401, {"error": {"message": "token missing"}})
            return
        prompt = body["messages"][-1]["content"]
        self.server.prompts.put(prompt)
        if prompt == "fail":
            self.reply(500, {"error": {"message": "intentional upstream failure"}})
            return
        if prompt == "interrupt":
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            self.wfile.write(b'data: {"choices":[{"delta":{"content":"partial"}}]}\n\n')
            self.wfile.flush()
            self.server.stream_started.set()
            self.server.release_stream.wait(timeout=10)
            return
        data = {"choices": [{"delta": {"content": f"reply: {prompt}"}}]}
        payload = f"data: {json.dumps(data, ensure_ascii=False)}\n\ndata: [DONE]\n\n".encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


@contextlib.contextmanager
def mock_node(require_token=True):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    server.require_token = require_token
    server.prompts = queue.Queue()
    server.stream_started = threading.Event()
    server.release_stream = threading.Event()
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield server, f"http://127.0.0.1:{server.server_port}"
    finally:
        server.release_stream.set()
        server.shutdown()
        server.server_close()
        thread.join()


def run(binary, args, env):
    return subprocess.run([str(binary), *args], env=env, text=True, capture_output=True, timeout=12)


def interruption_check(binary, url, env, server):
    process = subprocess.Popen([str(binary), "--connect", url, "--once", "interrupt"],
                               env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        assert server.stream_started.wait(timeout=5), "stream did not start"
        process.terminate()
        output, error = process.communicate(timeout=5)
        assert process.returncode != 0, f"interrupted reply was reported as successful: {output!r}"
        assert "interrupted" in error, error
        print("PASS interrupted single-shot replies return a failure exit code")
    finally:
        server.release_stream.set()
        if process.poll() is None:
            process.kill()
        process.wait(timeout=3)


def pty_check(binary, url, env, server):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))

    def controlling_terminal():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    process = subprocess.Popen([str(binary), "--connect", url], stdin=slave,
                               stdout=slave, stderr=slave, env={**env, "TERM": "xterm-256color"},
                               preexec_fn=controlling_terminal)
    os.close(slave)
    output = bytearray()

    def wait_for(condition):
        deadline = time.monotonic() + 6
        while time.monotonic() < deadline:
            if condition():
                return
            if select.select([master], [], [], 0.1)[0]:
                try:
                    output.extend(os.read(master, 65536))
                except OSError:
                    if condition():
                        return
                    time.sleep(0.02)
        raise AssertionError(f"terminal did not reach expected state: {output.decode(errors='replace')}")

    try:
        wait_for(lambda: b"chat" in output)
        os.write(master, b"deep prompt\r")
        wait_for(lambda: not server.prompts.empty())
        assert server.prompts.get_nowait() == "deep prompt", "ordinary d/p keys were intercepted"
        os.write(master, b"\x1b")
        wait_for(lambda: process.poll() is not None)
        assert process.returncode == 0, output.decode(errors="replace")
    finally:
        if process.poll() is None:
            process.kill()
        process.wait(timeout=3)
        os.close(master)


def managed_tui_check(binary, home):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 140, 0, 0))

    def controlling_terminal():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    env = {**os.environ, "LOCALHOSTAI_HOME": str(Path(home).resolve()), "TERM": "xterm-256color"}
    process = subprocess.Popen([str(binary), "--no-lan", "--port", "18766"], stdin=slave,
                               stdout=slave, stderr=slave, env=env, preexec_fn=controlling_terminal)
    os.close(slave)
    output = bytearray()

    def wait_for(text, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if text in output:
                return
            if select.select([master], [], [], 0.1)[0]:
                try:
                    output.extend(os.read(master, 65536))
                except OSError:
                    pass
        raise AssertionError(f"managed TUI did not show {text!r}: {output.decode(errors='replace')[-4000:]}")

    try:
        wait_for(b"chat")
        os.write(master, b"\x04")  # Ctrl+D
        wait_for(b"resource controls")
        wait_for(b"shared cap")
        os.write(master, b" ")
        wait_for(b"excluded")
        os.write(master, b" ")
        time.sleep(1)
        os.write(master, b"\x1b")
        os.write(master, b"\x1b[A\r")
        wait_for(b"serving qwen2.5-0.5b-instruct-q4_k_m", 120)
        os.write(master, b"Reply with one word: ready\r")
        wait_for(b"streaming", 120)
        os.write(master, b"\x1b")
        deadline = time.monotonic() + 5
        while process.poll() is None and time.monotonic() < deadline:
            time.sleep(0.05)
        assert process.returncode == 0, output.decode(errors="replace")[-4000:]
        print("PASS managed TUI loads the installed GGUF and chats without Ollama")
    finally:
        if process.poll() is None:
            process.kill()
        process.wait(timeout=3)
        os.close(master)


def first_run_catalog_check(binary, home):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 140, 0, 0))

    def controlling_terminal():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    process = subprocess.Popen([str(binary), "--no-lan", "--port", "18767"], stdin=slave,
                               stdout=slave, stderr=slave,
                               env={**os.environ, "LOCALHOSTAI_HOME": home, "TERM": "xterm-256color"},
                               preexec_fn=controlling_terminal)
    os.close(slave)
    output = bytearray()
    try:
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and b"Qwen 2.5 1.5B Instruct" not in output:
            if select.select([master], [], [], 0.1)[0]:
                try:
                    output.extend(os.read(master, 65536))
                except OSError:
                    pass
        assert b"Qwen 2.5 1.5B Instruct" in output and b"recommended" in output, output.decode(errors="replace")
        os.write(master, b"\x1b")
        time.sleep(0.2)
        os.write(master, b"\x04")  # Ctrl+D
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and b"resource controls" not in output:
            if select.select([master], [], [], 0.1)[0]:
                try:
                    output.extend(os.read(master, 65536))
                except OSError:
                    pass
        assert b"resource controls" in output and b"shared cap" in output, output.decode(errors="replace")
        os.write(master, b" ")
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and b"excluded" not in output:
            if select.select([master], [], [], 0.1)[0]:
                try:
                    output.extend(os.read(master, 65536))
                except OSError:
                    pass
        assert b"excluded" in output, output.decode(errors="replace")
        os.write(master, b" ")
        time.sleep(0.5)
        os.write(master, b"\x1b")
        time.sleep(0.2)
        os.write(master, b"\x1b")
        deadline = time.monotonic() + 5
        while process.poll() is None and time.monotonic() < deadline:
            time.sleep(0.05)
        assert process.returncode == 0, output.decode(errors="replace")
        print("PASS first run opens the self-contained model catalog")
    finally:
        if process.poll() is None:
            process.kill()
        process.wait(timeout=3)
        os.close(master)


def rpc_check(llama_server, rpc_server):
    processes = []
    peers = []
    try:
        for _ in range(2):
            with socket.socket() as reservation:
                reservation.bind(("127.0.0.1", 0))
                port = reservation.getsockname()[1]
            process = subprocess.Popen([rpc_server, "-H", "127.0.0.1", "-p", str(port), "-t", "1", "--device", "CPU"],
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(process)
            deadline = time.monotonic() + 5
            while True:
                assert process.poll() is None, "native RPC worker exited"
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                        break
                except OSError:
                    assert time.monotonic() < deadline, "native RPC worker did not listen"
                    time.sleep(0.05)
            peers.append(f"127.0.0.1:{port}")
        result = subprocess.run([llama_server, "--rpc", ",".join(peers), "--device", "RPC0,RPC1",
                                 "--split-mode", "layer", "--tensor-split", "2,1", "-ngl", "999", "--list-devices"],
                                capture_output=True, text=True, timeout=12)
        assert result.returncode == 0, result.stderr
        assert "RPC0" in result.stdout and "RPC1" in result.stdout, result.stdout
        print("PASS native llama.cpp accepts the selected RPC devices and split flags")
    finally:
        for process in processes:
            process.terminate()
        for process in processes:
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/localhostai")
    parser.add_argument("--llama-server")
    parser.add_argument("--rpc-server")
    parser.add_argument("--managed-home", help="existing isolated home containing an installed catalog model")
    options = parser.parse_args()
    binary = Path(options.binary).resolve()
    assert binary.is_file(), "build the CLI with cargo build --workspace first"
    with tempfile.TemporaryDirectory(prefix="localhostai-check-", dir=os.environ.get("LOCALHOSTAI_TEST_TMPDIR")) as temp:
        env = {**os.environ, "LOCALHOSTAI_HOME": temp, "LOCALHOSTAI_TOKEN": "MixedCaseToken"}
        invalid = run(binary, ["--port", "not-a-port"], env)
        assert invalid.returncode != 0 and not (Path(temp) / "config.toml").exists()
        print("PASS invalid arguments fail before creating node state")
        first_run_catalog_check(binary, temp)
        with mock_node() as (server, url):
            result = run(binary, ["--connect", url, "--once", "d p 你好"], env)
            assert result.returncode == 0 and result.stdout.strip() == "reply: d p 你好", (result.stdout, result.stderr)
            result = run(binary, ["--connect", url, "--once", "fail"], env)
            assert result.returncode != 0 and "intentional upstream failure" in result.stderr, result.stderr
            result = run(binary, ["--connect", url, "--status"], env)
            assert result.returncode == 0 and "mock" in result.stdout, result.stderr
            print("PASS remote --once/--status preserve authentication and failure exit codes")
            interruption_check(binary, url, env, server)
            while not server.prompts.empty():
                server.prompts.get_nowait()
            pty_check(binary, url, env, server)
            print("PASS terminal typing preserves d/p and Esc exits without hanging")
        with mock_node(require_token=False) as (_, url):
            config = f'engine = "ollama"\nollama_url = "{url}/v1"\nnode_name = \'Studio #1 "laptop"\'\n'
            (Path(temp) / "config.toml").write_text(config)
            result = run(binary, ["--no-lan", "--require-pairing", "--once", "local"], env)
            assert result.returncode == 0 and result.stdout.strip() == "reply: local", result.stderr
            assert (Path(temp) / "pair_token").stat().st_mode & 0o777 == 0o600
            assert all((Path(temp) / subdir).is_dir() for subdir in ("models", "bin", "logs"))
            result = run(binary, ["--no-lan", "--status"], env)
            assert result.returncode == 0 and 'Studio #1 "laptop"' in result.stdout, result.stderr
            print("PASS embedded paired chat, private token storage, directories, and quoted configuration")
    if options.llama_server and options.rpc_server:
        rpc_check(options.llama_server, options.rpc_server)
    if options.managed_home:
        managed_tui_check(binary, options.managed_home)


if __name__ == "__main__":
    main()
