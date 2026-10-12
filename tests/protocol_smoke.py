"""Exercise the real stdio server with isolated system commands, never WeChat."""

import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time


def check_peer_lock(binary, env, release):
    peer = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.DEVNULL, text=True, env=env)
    output = queue.Queue()
    def read():
        for line in peer.stdout:
            output.put(json.loads(line))
    thread = threading.Thread(target=read, daemon=True)
    thread.start()
    def send(request):
        peer.stdin.write(json.dumps(request)+'\n');peer.stdin.flush()
    try:
        send({'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2024-11-05'}})
        assert output.get(timeout=2)['id'] == 1
        send({'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'focus_wechat','arguments':{}}})
        send({'jsonrpc':'2.0','id':3,'method':'ping'})
        assert output.get(timeout=2)['id'] == 3, 'Peer executed UI while another process owned the desktop'
        try:
            response = output.get(timeout=0.15)
            raise AssertionError(('UI lock was not exclusive',response))
        except queue.Empty:
            pass
        release()
        response = output.get(timeout=3)
        assert response['id'] == 2 and response['result']['isError'] is False, response
        peer.stdin.close()
        assert peer.wait(timeout=3) == 0
        thread.join(timeout=1)
    finally:
        if peer.poll() is None:
            peer.kill();peer.wait()


def run(binary, directory, version):
    env = dict(os.environ)
    env["MESSAGE_TEST_HOLD"] = str(directory / ("hold-" + version))
    env["PATH"] = str(directory) + os.pathsep + env["PATH"]
    env["MESSAGE_GATEWAY_DB"] = str(directory / "service.sqlite3")
    env["MESSAGE_GATEWAY_CONFIG"] = str(directory / "accounts.json")
    process = subprocess.Popen(
        [str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, text=True, env=env,
    )
    output = queue.Queue()

    def read_output():
        for line in process.stdout:
            output.put(json.loads(line))

    thread = threading.Thread(target=read_output, daemon=True)
    thread.start()

    def send(method, request_id=None, params=None):
        request = {"jsonrpc": "2.0", "method": method}
        if request_id is not None:
            request["id"] = request_id
        if params is not None:
            request["params"] = params
        process.stdin.write(json.dumps(request) + "\n")
        process.stdin.flush()

    def receive(request_id, timeout=2):
        response = output.get(timeout=timeout)
        assert response["id"] == request_id, response
        return response

    try:
        send("initialize", 1, {
            "protocolVersion": version, "capabilities": {},
            "clientInfo": {"name": "isolated-audit", "version": "1"},
        })
        assert receive(1)["result"]["protocolVersion"] == version
        send("notifications/initialized")
        send("tools/call", 2, {"name": "list_accounts", "arguments": {}})
        result = receive(2)["result"]
        assert ("structuredContent" in result) == (version == "2025-06-18")
        data = json.loads(result["content"][0]["text"])
        assert data["accounts"][0]["key"] == {"platform": "wechat", "account_id": "local"}

        send("tools/call", 3, {
            "name": "wait_for_message", "arguments": {
                "platform": "wechat", "account_id": "local", "timeout_seconds": 60,
            },
        })
        deadline = time.monotonic()+3
        while not Path(env['MESSAGE_TEST_HOLD']).exists() and time.monotonic()<deadline:
            time.sleep(0.02)
        assert Path(env['MESSAGE_TEST_HOLD']).exists(), 'Isolated UI operation did not acquire the desktop lock'
        start = time.monotonic()
        send("ping", 4)
        assert receive(4)["result"] == {}
        assert time.monotonic() - start < 1, "ping blocked by UI polling"
        check_peer_lock(binary,env,lambda:send("notifications/cancelled", params={"requestId": 3}))
        send("tools/call", 5, {
            "name": "list_conversations", "arguments": {"platform": "wechat", "account_id": "local"},
        })
        # The simulated screenshot fails. Reaching this result proves cancellation
        # released the UI lock without mistaking a failed capture for success.
        assert receive(5)["result"]["isError"] is True
        send("unknown_method", 6)
        assert receive(6)["error"]["code"] == -32601
        process.stdin.write('{"jsonrpc":"9.9","id":7,"method":"ping"}\n')
        process.stdin.flush()
        assert receive(7)["error"]["code"] == -32600
        process.stdin.write('{\n')
        process.stdin.flush()
        assert receive(None)["error"]["code"] == -32700
        send("tools/call", 8, {
            "name":"wait_for_events", "arguments":{
                "platform":"wechat", "account_id":"local", "timeout_seconds":60,
            },
        })
        process.stdin.close()
        assert process.wait(timeout=3) == 0
        thread.join(timeout=1)
        assert output.empty(), "cancelled request unexpectedly produced a response"
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()


def main():
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/wechat-mcp").resolve()
    with tempfile.TemporaryDirectory(prefix="message-gateway-test-") as temp:
        directory = Path(temp)
        for name, script in {
            "osascript": '#!/bin/sh\nscript=$(/bin/cat)\ncase "$script" in *"set frontmost"*) if [ ! -f "$MESSAGE_TEST_HOLD" ]; then : > "$MESSAGE_TEST_HOLD"; exec /bin/sleep 60; fi;; esac\nprintf "true\\n"\n',
            "screencapture": "#!/bin/sh\nexit 1\n",
        }.items():
            path = directory / name
            path.write_text(script)
            path.chmod(0o700)
        (directory / "accounts.json").write_text(json.dumps({"accounts": [{
            "platform": "wechat", "account_id": "local", "adapter": "wechat_desktop",
        }]}))
        for version in ["2024-11-05", "2025-06-18"]:
            run(binary, directory, version)
    print("PASS: real stdio routing, protocol versions, nonblocking ping, cross-process desktop exclusion, cancellation and errors (simulated desktop).")


if __name__ == "__main__":
    main()
