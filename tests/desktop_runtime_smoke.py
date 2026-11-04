"""Exercise native-stream routing against an isolated fake helper, never WeChat."""
import json
import os
import queue
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading


HELPER = r'''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
counter = Path(os.environ["HELPER_STARTS"])
counter.write_text(str(int(counter.read_text()) + 1) if counter.exists() else "1")
sequence = 0
search_reads = 0
for line in sys.stdin:
    request = json.loads(line)
    state = Path(os.environ['OVERLAY_STATE'])
    assert not state.exists() or state.read_text() == 'hide', 'overlay visible during native capture'
    sequence += 1
    mode = os.environ.get("HELPER_CASE", "normal")
    result = {"protocol_version": 1, "id": request["id"], "ok": True}
    if mode == "wrong_id": result["id"] += 100
    if mode == "failed":
        result.update(ok=False, error="capture_unavailable")
    elif mode == "oversized":
        print("x" * (4 * 1024 * 1024 + 10), flush=True)
        continue
    else:
        def text(value, x, y):
            return {"text": value, "bounds": {"x": x, "y": y, "width": .15, "height": .03}, "confidence": .95}
        lines = [text("Alice", .1, .1), text("12:00", .8, .1), text("preview", .1, .14)]
        if request["params"]["region"] == "chat": lines = [text("收到", .1, .1), text("收到", .1, .3)]
        if request["params"]["region"] == "title": lines = [text("Alice", .1, .1)]
        if (mode.startswith("search_") or mode == "duplicate_target") and request["params"]["region"] == "title":
            clicks = Path(os.environ["HELPER_CLICKS"])
            clicked = len(clicks.read_text().splitlines()) if clicks.exists() else 0
            lines = [text("Alice" if clicked >= 2 else "Other", .1, .1)]
        if mode.startswith("search_"):
            if request["params"]["region"] == "search":
                search_reads += 1
                lines = [text("搜索" if search_reads == 1 else ("Alice" if mode == "search_success" else "Wrong"), .1, .1)]
            elif request["params"]["region"] == "sessions" and search_reads < 2:
                lines = [text("Other", .1, .1)]
        if mode == "duplicate_target" and request["params"]["region"] == "sessions":
            lines = [text("Alice",.1,.1),text("Alice",.1,.4)]
        if mode == "blank_window": lines = []
        result["result"] = {
            "epoch": "fixture-epoch", "sequence": sequence,
            "window": {"window_id": 1, "pid": 123, "x": 0, "y": 0, "width": 950, "height": 680, "on_screen": True},
            "region": {"x": .05, "y": .1, "width": .25, "height": .8},
            "digest": "a" * 64, "captured_at_ms": 1000,
            "ocr_cached": sequence > 1, "lines": lines, "title_lines": [text("Alice", .1, .1)]}
        if mode == "bad_bounds": result["result"]["lines"][0]["bounds"]["width"] = 2
        if mode == "changed_window": result["result"]["window"]["window_id"] = 2
    print(json.dumps(result, ensure_ascii=False), flush=True)
'''

LEGACY_OCR = r'''#!/usr/bin/env python3
import json,os,sys
from pathlib import Path
name = Path(sys.argv[1]).name
mode = os.environ['HELPER_CASE']
def count(key):
    path = Path(os.environ['HELPER_STARTS'] + key)
    value = int(path.read_text())+1 if path.exists() else 1
    path.write_text(str(value))
    return value
if 'search' in name:
    index = count('search')
    text = 'Q 搜察' if index == 1 else ('Wrong' if mode == 'legacy_query_mismatch' else 'Alice')
elif 'list' in name:
    text = 'Other' if count('list') == 1 else 'Alice'
elif 'title' in name:
    clicks = Path(os.environ['HELPER_CLICKS'])
    text = 'Alice' if clicks.exists() and len(clicks.read_text().splitlines()) >= 2 else 'Other'
else:
    text = 'visible text'
items = [{'text':text,'x':0.1,'y':0.1}]
if mode == 'legacy_duplicate' and 'list' in name and text == 'Alice':
    items.append({'text':'Alice','x':0.1,'y':0.4})
print(json.dumps(items,ensure_ascii=False))
'''

LEGACY_CAPTURE = r'''#!/usr/bin/env python3
import os,struct,sys,zlib
from pathlib import Path
state = Path(os.environ['OVERLAY_STATE'])
assert not state.exists() or state.read_text() == 'hide', 'overlay visible during screen capture'
def chunk(kind,data):
    return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data)&0xffffffff)
png=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',950,680,8,2,0,0,0))
png+=chunk(b'IDAT',zlib.compress((b'\0'+b'\xff'*(950*3))*680))+chunk(b'IEND',b'')
Path(sys.argv[-1]).write_bytes(png)
'''


def run_case(binary, directory, mode):
    case = directory / mode
    case.mkdir()
    (case / "bin").mkdir()
    executable = case / "wechat-mcp"
    shutil.copy2(binary, executable)
    scripts = {
        "helper": HELPER,
        "wechat_overlay": '''#!/usr/bin/env python3
import json,os,sys
from pathlib import Path
for line in sys.stdin:
    event = json.loads(line)['event']
    Path(os.environ['OVERLAY_STATE']).write_text(event)
    with open(os.environ['OVERLAY_EVENTS'], 'a') as log: log.write(event+'\\n')
    print('{"ok":true}',flush=True)
''',
        "osascript": "#!/bin/sh\n/bin/cat >/dev/null\nprintf 'true\\n'\n",
        "screencapture": "#!/bin/sh\nexit 0\n",
        "pbpaste": "#!/bin/sh\nexit 0\n",
        "pbcopy": "#!/bin/sh\n/bin/cat >/dev/null\n",
        "bin/wechat_click": "#!/bin/sh\nprintf '%s %s\\n' \"$1\" \"$2\" >> \"$HELPER_CLICKS\"\n",
        "bin/wechat_ocr": "#!/bin/sh\nprintf '%s\\n' '[{\"text\":\"Alice\",\"x\":0.1,\"y\":0.1}]'\n",
        "bin/wechat_window": """#!/bin/sh
if [ "$1" = "--screens" ]; then
    printf '%s\n' '[{"x":0,"y":0,"width":1920,"height":1080}]'
else
    printf '%s\n' '{"window_id":1,"x":0,"y":0,"width":950,"height":680,"on_screen":true}'
fi
""",
    }
    legacy_case = mode.startswith('legacy_')
    if legacy_case:
        scripts['bin/wechat_ocr'] = LEGACY_OCR
        scripts['screencapture'] = LEGACY_CAPTURE
    for name, source in scripts.items():
        path = case / name
        path.write_text(source)
        path.chmod(0o700)
    (case / "accounts.json").write_text(json.dumps({"accounts": [
        {"platform": "wechat", "account_id": "fixture", "adapter": "wechat_desktop"}]}))
    env = dict(os.environ)
    env.update(WECHAT_MCP_DESKTOP_RUNTIME="stream", WECHAT_MCP_DESKTOP_HELPER=str(case / "helper"),
               HELPER_CASE=mode, HELPER_STARTS=str(case / "starts"),
               HELPER_CLICKS=str(case / "clicks"),
               WECHAT_MCP_OPERATION_OVERLAY="1", OVERLAY_STATE=str(case / "overlay-state"),
               OVERLAY_EVENTS=str(case / "overlay-events"),
               MESSAGE_GATEWAY_DB=str(case / "service.sqlite3"), MESSAGE_GATEWAY_CONFIG=str(case / "accounts.json"),
               PATH=str(case) + os.pathsep + env["PATH"])
    if legacy_case:
        env['WECHAT_MCP_DESKTOP_RUNTIME'] = 'legacy'
    requests = []
    for number, tool in enumerate(["list_conversations", "list_conversations", "get_messages"], 1):
        requests.append({"jsonrpc": "2.0", "id": number, "method": "tools/call", "params": {
            "name": tool, "arguments": {"platform": "wechat", "account_id": "fixture"}}})
    navigation_case = legacy_case or mode.startswith("search_") or mode in ["duplicate_target", "blank_window"]
    if navigation_case:
        requests = [{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"get_messages","arguments":{"platform":"wechat","account_id":"fixture","recipient":"Alice"}}}]
    # Request one by one: deterministic ordering also checks that a complete reply is reusable.
    process = subprocess.Popen([str(executable)], cwd=case, env=env, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    received = queue.Queue()
    def read_output():
        for line in process.stdout:
            received.put(json.loads(line))
    reader = threading.Thread(target=read_output, daemon=True)
    reader.start()
    try:
        responses = []
        for request in requests:
            process.stdin.write(json.dumps(request) + "\n")
            process.stdin.flush()
            responses.append(received.get(timeout=10))
        if mode == "normal":
            def tool(number, name, arguments):
                request = {"jsonrpc":"2.0","id":number,"method":"tools/call","params":{
                    "name":name,"arguments":{"platform":"wechat","account_id":"fixture",**arguments}}}
                process.stdin.write(json.dumps(request) + "\n")
                process.stdin.flush()
                response = received.get(timeout=10)
                assert response["id"] == number, response
                result = response["result"]
                assert result["isError"] is False, result
                return json.loads(result["content"][0]["text"])["data"]
            customer = tool(4,"create_customer",{
                "idempotency_key":"fixture-customer","display_name":"Alice","recipient":"Alice"})
            first = tool(5,"capture_customer_context",{"customer_id":customer["customer_id"]})
            second = tool(6,"capture_customer_context",{"customer_id":customer["customer_id"]})
            assert first["payload"]["text_continuity"]["state"] == "baseline", first
            continuity = second["payload"]["text_continuity"]
            assert continuity["previous_event_id"] == first["event_id"], second
            assert continuity["state"] == "unchanged" and continuity["new_lines"] == [], second
            assert continuity["message_continuity_verified"] is False, second
            assert second["state"] == "recorded" and second["verified"] is False, second
        process.stdin.close()
        assert process.wait(timeout=5) == 0, process.stderr.read()
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
    if navigation_case:
        success = mode in ["search_success", "legacy_success"]
        assert responses[0]["result"]["isError"] is not success, responses
        clicks = (case / "clicks").read_text().splitlines() if (case / "clicks").exists() else []
        assert len(clicks) == {"search_success":2,"search_readback_mismatch":1,"duplicate_target":0,"blank_window":0,
                              "legacy_success":2,"legacy_query_mismatch":1,"legacy_duplicate":1}[mode], clicks
        if clicks:
            events = (case / 'overlay-events').read_text().splitlines()
            assert events.count('move') == len(clicks) and events.count('click') == len(clicks), events
        if mode == "blank_window":
            assert "无可读文字" in responses[0]["result"]["content"][0]["text"], responses
        if legacy_case:
            assert not (case / 'starts').exists(), 'Legacy navigation started the stream helper'
        else:
            assert (case / "starts").read_text() == "1", "Native navigation unexpectedly restarted helper"
    elif mode == "normal":
        assert all(response["result"].get("isError") is False for response in responses), responses
        outputs = [json.loads(response["result"]["content"][0]["text"]) for response in responses]
        assert outputs[0]["data"]["conversations"][0]["display_name"] == "Alice", outputs
        assert outputs[0]["data"]["conversations"][0]["preview"] == "preview", outputs
        assert outputs[1]["data"]["capture"]["sequence"] == 2, outputs
        assert outputs[2]["data"]["text_lines"] == ["收到", "收到"], outputs
        assert outputs[2]["data"]["observation"]["kind"] == "ocr_text_lines", outputs
        assert outputs[2]["data"]["direction"] == "unknown", outputs
        assert (case / "starts").read_text() == "1", "Helper process was not reused"
    else:
        assert all(response["result"].get("isError") is True for response in responses), responses
        if mode not in ["changed_window"]:
            assert (case / "starts").read_text() == "3", "Failed helper session was reused"


def main():
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/wechat-mcp").resolve()
    with tempfile.TemporaryDirectory(prefix="wechat-native-routing-") as temp:
        for case in ["normal", "wrong_id", "failed", "oversized", "bad_bounds", "changed_window",
                     "search_success", "search_readback_mismatch", "duplicate_target", "blank_window",
                     "legacy_success", "legacy_query_mismatch", "legacy_duplicate"]:
            run_case(binary, Path(temp), case)
    print("PASS: native stream routing, durable customer captures, child reuse, duplicate lines, protocol failures, bounds and window epoch checks (simulated desktop).")


if __name__ == "__main__":
    main()
