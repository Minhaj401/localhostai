#!/usr/bin/env python3
"""Exercise the phone entry point's real Linux signal/parent-lifetime behavior."""
import argparse
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import socket


def alive(pid):
    try:
        return Path(f"/proc/{pid}/stat").read_text().rsplit(") ", 1)[1].split()[0] not in {"Z", "X"}
    except FileNotFoundError:
        return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--temp-dir", default="/tmp/opencode")
    options = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix="worker-lifecycle-", dir=options.temp_dir) as directory:
        directory = Path(directory)
        stub = directory / "rpc-stub.cpp"
        stub.write_text(r'''
#include <cerrno>
#include <cstdio>
#include <cstring>
#include <unistd.h>
int localhostai_rpc_main(int argc, char ** argv) {
    if (argc > 1 && std::strcmp(argv[1], "pipe") == 0) {
        int fds[2];
        if (pipe(fds) != 0) return 2;
        close(fds[0]);
        const int result = write(fds[1], "x", 1);
        close(fds[1]);
        return result == -1 && errno == EPIPE ? 0 : 3;
    }
    puts("ready");
    fflush(stdout);
    for (;;) pause();
}
''')
        binary = directory / "guard-check"
        subprocess.run([os.environ.get("CXX", "c++"), "-std=c++17", "-pthread",
                        str(root / "phone-rpc/worker-lifecycle.cpp"), str(stub), "-o", str(binary)], check=True)
        subprocess.run([str(binary), "pipe"], check=True)
        print("PASS disconnected clients do not kill the worker with SIGPIPE", flush=True)

        # The shipped wrapper exposes only the paired coordinator address and
        # keeps the upstream RPC listener on loopback.
        def free_port():
            with socket.socket() as reservation:
                reservation.bind(("127.0.0.1", 0))
                return reservation.getsockname()[1]
        external, internal = free_port(), free_port()
        gated = subprocess.Popen([str(binary), "--host", "0.0.0.0", "--port", str(external),
                                  "--allow-peer", "192.0.2.10", "--upstream-port", str(internal)],
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            assert gated.stdout.readline().strip() == "ready"
            deadline = time.monotonic() + 3
            while True:
                try:
                    with socket.create_connection(("127.0.0.1", external), timeout=.2) as client:
                        client.sendall(b"rejected")
                    break
                except OSError:
                    assert time.monotonic() < deadline
                    time.sleep(.05)
            assert gated.poll() is None
            print("PASS authenticated gate rejects a non-coordinator source without killing the worker", flush=True)
        finally:
            gated.kill()
            gated.wait(timeout=5)

        # Dart can start a child from a reusable/short-lived runtime thread.
        # Ending that thread must not be mistaken for the app process exiting.
        owner_code = '''import subprocess, sys, threading
def launch():
    child = subprocess.Popen([sys.argv[1]], stdout=subprocess.PIPE, text=True)
    assert child.stdout.readline().strip() == "ready"
    print(child.pid, flush=True)
thread = threading.Thread(target=launch)
thread.start()
thread.join()
sys.stdin.read()
'''
        for termination in (signal.SIGTERM, signal.SIGKILL):
            owner = subprocess.Popen([sys.executable, "-c", owner_code, str(binary)],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
            child_pid = int(owner.stdout.readline())
            try:
                time.sleep(0.2)
                assert alive(child_pid), "The worker died with its spawning thread while the app process was still alive"
                owner.send_signal(termination)
                owner.wait(timeout=5)
                for _ in range(50):
                    if not alive(child_pid):
                        break
                    time.sleep(0.1)
                assert not alive(child_pid), "Worker survived its owning app process"
                print(f"PASS spawning-thread exit is harmless; owner {termination.name} releases the worker", flush=True)
            finally:
                if owner.poll() is None:
                    owner.kill()
                owner.wait(timeout=5)
                if alive(child_pid):
                    os.kill(child_pid, signal.SIGKILL)


if __name__ == "__main__":
    main()
