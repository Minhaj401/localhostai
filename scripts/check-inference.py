#!/usr/bin/env python3
"""Real model load/reload and RPC failure checks; uses only private test processes.

Requires an existing GGUF and matching native llama.cpp binaries. RPC workers
run on loopback as phone stand-ins; this does not substitute for Android tests.
"""
import argparse
import contextlib
import json
import os
from pathlib import Path
import re
import resource
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def request(base, path, body=None):
    req = urllib.request.Request(base + path,
        data=None if body is None else json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Authorization": "Bearer integration-token"})
    try:
        with urllib.request.urlopen(req, timeout=210) as response:
            return response.status, json.loads(response.read())
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read())


def require(base, path, body=None):
    status, value = request(base, path, body)
    assert 200 <= status < 300, (path, status, value)
    return value


def no_core_dump():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def spawn(args, log, env=None):
    return subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT,
        env=env, preexec_fn=no_core_dump)


def stop(process):
    if process.poll() is None:
        process.terminate()
    try:
        process.wait(timeout=8)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def process_running(pid):
    try:
        os.kill(pid, 0)
        status = Path(f"/proc/{pid}/stat")
        return not status.exists() or status.read_text().rsplit(") ", 1)[1].split()[0] not in {"Z", "X"}
    except ProcessLookupError:
        return False
    except FileNotFoundError:
        return False


def check_child_stopped(pid):
    try:
        for _ in range(50):
            if not process_running(pid):
                return
            time.sleep(0.1)
        raise AssertionError(f"llama-server {pid} survived termination of its owning node")
    finally:
        # Clean up this test's child even when testing a broken node binary.
        if process_running(pid):
            os.kill(pid, signal.SIGKILL)


def read_exact(sock, length):
    result = b""
    while len(result) < length:
        data = sock.recv(length - len(result))
        if not data:
            raise OSError("worker closed its RPC connection")
        result += data
    return result


def wait_worker(address, process):
    for _ in range(100):
        assert process.poll() is None, "RPC test worker exited early"
        try:
            with socket.create_connection(address, timeout=0.5) as sock:
                sock.sendall(struct.pack("<BQ", 14, 24) + bytes(24))
                size = struct.unpack("<Q", read_exact(sock, 8))[0]
                assert size == 28
                assert read_exact(sock, size)[:2] == bytes([8, 0])
                return
        except OSError:
            time.sleep(0.05)
    raise AssertionError("RPC worker did not become ready")


def reply(base):
    value = require(base, "/v1/chat/completions", {
        "model": "integration-model", "messages": [{"role": "user", "content": "Reply only with OK."}],
        "stream": False, "max_tokens": 8,
    })
    assert value["choices"][0]["message"]["content"].strip(), value


@contextlib.contextmanager
def poll_pool(base, workers):
    """Model startup must not race a poll/heartbeat for the single-client socket."""
    done = threading.Event()
    errors = []
    def poll():
        try:
            while not done.is_set():
                require(base, "/peers")
                for _, _, body, _ in workers:
                    require(base, "/peers", body)
                done.wait(0.1)
        except Exception as error:
            errors.append(error)
    thread = threading.Thread(target=poll, daemon=True)
    thread.start()
    try:
        yield
    finally:
        done.set()
        thread.join(timeout=10)
        assert not thread.is_alive(), "Pool refresh got stuck during model startup"
        assert not errors, errors


def multi_turn(base):
    messages = []
    for turn in range(12):
        # Vary prompt lengths to exercise RPC graph reuse and changed KV shapes.
        messages.append({"role": "user", "content": "Say OK. " + "test " * (turn * 3)})
        value = require(base, "/v1/chat/completions", {
            "model": "integration-model", "messages": messages,
            "stream": False, "max_tokens": 8,
        })
        answer = value["choices"][0]["message"]["content"]
        assert answer.strip(), value
        messages.append({"role": "assistant", "content": answer})


def check_placement(result):
    status = result["status"]
    text = Path(status["log_path"]).read_text(errors="replace")
    placements = {int(layer): device.rstrip(',') for layer, device in re.findall(
        r"layer\s+(\d+) assigned to device (\S+)", text)}
    total = result["plan"]["total_layers"]
    assert len([layer for layer in placements if layer < total]) == total, text[-3000:]
    remotes = [a for a in result["plan"]["assignments"] if a["rpc"] and a["layers"]]
    for index, assignment in enumerate(remotes):
        actual = sum(device == f"RPC{index}" for layer, device in placements.items() if layer < total)
        assert actual == assignment["layers"], (assignment, placements)
    if not remotes:
        assert all(device == "CPU" for layer, device in placements.items() if layer < total)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/privateml")
    parser.add_argument("--model", required=True)
    parser.add_argument("--llama-server", required=True)
    parser.add_argument("--rpc-server", required=True)
    parser.add_argument("--temp-dir", default="/tmp/opencode")
    options = parser.parse_args()
    binary = str(Path(options.binary).resolve())
    llama = str(Path(options.llama_server).resolve())
    rpc = str(Path(options.rpc_server).resolve())
    model = str(Path(options.model).resolve())
    with tempfile.TemporaryDirectory(prefix="privateml-inference-", dir=options.temp_dir) as temp, contextlib.ExitStack() as stack:
        home = Path(temp)
        (home / "config.toml").write_text('engine = "managed"\nrequire_pairing = true\ndiscovery = false\n')
        (home / "pair_token").write_text("integration-token")
        (home / "pair_token").chmod(0o600)
        if Path(model).stem in {"qwen2.5-0.5b-instruct-q4_k_m", "qwen2.5-1.5b-instruct-q4_k_m", "qwen2.5-3b-instruct-q4_k_m", "qwen2.5-7b-instruct-q3_k_m", "qwen2.5-coder-7b-instruct-q4_k_m"}:
            (home / "models").mkdir()
            (home / "models" / Path(model).name).symlink_to(model)
        # Debug logging lets the test compare actual block placement to the plan.
        wrapper = home / "llama-debug"
        pid_file = home / "llama.pid"
        wrapper.write_text(f"#!/usr/bin/env python3\nimport os, sys\nfrom pathlib import Path\nPath({str(pid_file)!r}).write_text(str(os.getpid()))\nos.execv({llama!r}, [{llama!r}, '-lv', '5', *sys.argv[1:]])\n")
        wrapper.chmod(0o700)
        env = {**os.environ, "PRIVATELM_HOME": temp, "PRIVATELM_LLAMA_SERVER": str(wrapper), "PRIVATELM_RPC_SERVER": rpc}
        base = f"http://127.0.0.1:{port()}"
        node_log = stack.enter_context((home / "node.log").open("w"))
        node = spawn([binary, "--headless", "--no-lan", "--port", base.rsplit(":", 1)[1]], node_log, env)
        stack.callback(stop, node)
        for _ in range(100):
            try:
                require(base, "/health")
                break
            except (OSError, urllib.error.URLError):
                time.sleep(0.05)
        if (home / "models" / Path(model).name).exists():
            installed = require(base, "/managed/install", {"model": Path(model).stem})
            assert installed["ok"]
            assert require(base, "/managed")["status"]["phase"] == "ready"
            print("PASS installed catalog model validates offline", flush=True)
        registered = require(base, "/models", {"name": "integration-model", "path": model})["model"]
        assert registered["layers"] > 0 and registered["kv_bytes_per_token"] > 0
        load = {"model": "integration-model", "ctx": 1024}
        local = require(base, "/cluster/start", load)
        assert local["ok"] and not local["status"]["peers"]
        check_placement(local)
        reply(base)
        print("PASS direct CPU model load, actual GGUF metadata, and inference", flush=True)

        # Coordinator-first planning only spills when the local budget is full.
        # Disable self and give each stand-in less than one model but enough in
        # aggregate, ensuring this check still exercises two RPC workers.
        runtime_hint = (Path(model).stat().st_size + 1024 * 1024 - 1) // (1024 * 1024) + 256
        worker_budget = 256 + (runtime_hint * 3 + 4) // 5
        worker_available = 900 + worker_budget
        workers = []
        for i in range(2):
            endpoint = ("127.0.0.1", port())
            log_path = home / f"rpc-{i}.log"
            log = stack.enter_context(log_path.open("w"))
            process = spawn([rpc, "-H", endpoint[0], "-p", str(endpoint[1]), "-t", "1", "--device", "CPU"], log)
            stack.callback(stop, process)
            wait_worker(endpoint, process)
            body = {"id": f"test-phone-{i}", "name": f"phone-{i}", "kind": "phone",
                    "ram_mb": max(3900, worker_available), "available_mb": worker_available,
                    "cores": 4, "arch": "arm64", "rpc": f"{endpoint[0]}:{endpoint[1]}"}
            require(base, "/peers", body)
            workers.append((process, endpoint, body, log_path))

        stack.callback(stop, node)
        self_id = next(device["id"] for device in require(base, "/peers")["devices"] if device["self"])
        require(base, "/peers/toggle", {"id": self_id, "enabled": False})

        with poll_pool(base, workers):
            mixed = require(base, "/cluster/start", load)
        assert mixed["ok"] and len(mixed["status"]["peers"]) == 2, mixed
        check_placement(mixed)
        reply(base)
        with poll_pool(base, workers):
            multi_turn(base)
        print("PASS multi-turn RPC graph reuse while polling and heartbeating", flush=True)
        print("PASS two-worker RPC load and exact planned transformer placement", flush=True)
        accepted_before = [p.read_text().count("Accepted client connection") for _, _, _, p in workers]
        for _ in range(12):
            peers = require(base, "/peers")["devices"]
            phones = [p for p in peers if not p["self"]]
            assert len(phones) == 2 and all(p["usable"] and p["in_use"] for p in phones), phones
        accepted_after = [p.read_text().count("Accepted client connection") for _, _, _, p in workers]
        assert accepted_before == accepted_after, (accepted_before, accepted_after)
        reply(base)
        with poll_pool(base, workers):
            reloaded = require(base, "/cluster/start", load)
        assert reloaded["ok"]
        check_placement(reloaded)
        reply(base)
        assert mixed["status"]["log_path"] != reloaded["status"]["log_path"]
        print("PASS repeated status polling, model replacement, and separate run logs", flush=True)

        require(base, "/cluster/stop", {})
        # Deliberately stop a worker while a real RPC-backed model is loaded.
        restarted = require(base, "/cluster/start", load)
        assert restarted["ok"]
        stop(workers[0][0])
        status, _ = request(base, "/v1/chat/completions", {
            "model": "integration-model", "messages": [{"role": "user", "content": "Hello"}], "max_tokens": 4,
        })
        assert status >= 400, status
        for _ in range(100):
            failed = require(base, "/cluster")
            if failed["state"] == "failed":
                break
            time.sleep(0.05)
        assert failed["state"] == "failed" and "RPC connection lost" in failed["error"], failed
        require(base, "/peers", {
            **{key: value for key, value in workers[0][2].items() if key != "rpc"},
            "worker": {"session": "exited-test-worker", "sequence": 1, "state": "error",
                       "exit_code": -9, "error": "Test worker exited with SIGKILL",
                       "log_tail": "retained native diagnostic"},
        })
        diagnosed = require(base, "/cluster")
        assert "SIGKILL" in diagnosed["error"], diagnosed
        assert "retained native diagnostic" in Path(diagnosed["log"]).read_text()
        assert require(base, "/health")["ok"] is False
        assert require(base, "/v1/models")["data"] == []
        print("PASS worker loss surfaces an actionable failure", flush=True)
        require(base, "/peers/toggle", {"id": workers[0][2]["id"], "enabled": False})
        remaining = {**workers[1][2], "ram_mb": 900 + runtime_hint * 2,
                     "available_mb": 900 + runtime_hint * 2}
        require(base, "/peers", remaining)
        recovered = require(base, "/cluster/start", load)
        assert recovered["ok"]
        reply(base)
        print("PASS recovery using the remaining worker", flush=True)
        # Terminating the node must release the loaded model without an explicit
        # /cluster/stop request, as when shutting down an SSH session or service.
        child_pid = int(pid_file.read_text())
        stop(node)
        check_child_stopped(child_pid)
        assert node.returncode == 0, node.returncode
        print("PASS node termination releases the native model process", flush=True)

        signals = [signal.SIGHUP]
        if sys.platform == "linux":
            signals.append(signal.SIGKILL)
        for shutdown in signals:
            node = spawn([binary, "--headless", "--no-lan", "--port", base.rsplit(":", 1)[1]], node_log, env)
            stack.callback(stop, node)
            for _ in range(100):
                try:
                    require(base, "/health")
                    break
                except (OSError, urllib.error.URLError):
                    time.sleep(0.05)
            require(base, "/models", {"name": "integration-model", "path": model})
            assert require(base, "/cluster/start", load)["ok"]
            child_pid = int(pid_file.read_text())
            node.send_signal(shutdown)
            node.wait(timeout=10)
            check_child_stopped(child_pid)
            assert node.returncode == (-signal.SIGKILL if shutdown == signal.SIGKILL else 0), node.returncode
            print(f"PASS {shutdown.name} leaves no native model process", flush=True)


if __name__ == "__main__":
    main()
