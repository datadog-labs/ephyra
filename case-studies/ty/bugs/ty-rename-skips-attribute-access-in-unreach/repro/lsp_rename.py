#!/usr/bin/env python3
"""Minimal LSP client: ask a language server to rename (or find references of) the symbol at a
position, print the locations it returns, and optionally write the renamed file.

usage: lsp_rename.py <file.py> <line> <col> <new_name> [--out renamed.py] -- <server command...>
  line/col are 0-based (LSP convention).
  examples:  -- ty server        -- pyright-langserver --stdio
"""
import json, os, subprocess, sys, threading, time
from pathlib import Path

args = sys.argv[1:]
sep = args.index("--")
opts, cmd = args[:sep], args[sep + 1:]
path, line, col, new = Path(opts[0]).resolve(), int(opts[1]), int(opts[2]), opts[3]
out = opts[opts.index("--out") + 1] if "--out" in opts else None
root = path.parent
text = path.read_text()

p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, cwd=root)
responses, lock = {}, threading.Condition()


def send(msg):
    body = json.dumps(msg).encode()
    p.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    p.stdin.flush()


def reader():
    while True:
        headers = {}
        while True:
            h = p.stdout.readline()
            if not h:
                return
            h = h.decode().strip()
            if not h:
                break
            k, v = h.split(":", 1)
            headers[k.lower()] = v.strip()
        msg = json.loads(p.stdout.read(int(headers["content-length"])))
        if "id" in msg and "method" in msg:  # server->client request: answer with null/empty
            reply = None
            if msg["method"] == "workspace/configuration":
                reply = [{} for _ in msg["params"]["items"]]
            send({"jsonrpc": "2.0", "id": msg["id"], "result": reply})
        elif "id" in msg:
            with lock:
                responses[msg["id"]] = msg
                lock.notify_all()


threading.Thread(target=reader, daemon=True).start()
next_id = [0]


def request(method, params, timeout=60):
    next_id[0] += 1
    i = next_id[0]
    send({"jsonrpc": "2.0", "id": i, "method": method, "params": params})
    with lock:
        lock.wait_for(lambda: i in responses, timeout)
    return responses.get(i, {}).get("result")


uri = path.as_uri()
request("initialize", {"processId": os.getpid(), "rootUri": root.as_uri(),
                       "workspaceFolders": [{"uri": root.as_uri(), "name": "w"}],
                       "capabilities": {"workspace": {"configuration": True, "workspaceEdit": {"documentChanges": False}}}})
send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
send({"jsonrpc": "2.0", "method": "textDocument/didOpen",
      "params": {"textDocument": {"uri": uri, "languageId": "python", "version": 1, "text": text}}})
time.sleep(1)
pos = {"textDocument": {"uri": uri}, "position": {"line": line, "character": col}}
refs = request("textDocument/references", {**pos, "context": {"includeDeclaration": True}}) or []
edit = request("textDocument/rename", {**pos, "newName": new}) or {}
send({"jsonrpc": "2.0", "id": 9999, "method": "shutdown"})
p.terminate()

lines = text.splitlines(keepends=True)
print("references:", sorted((r["range"]["start"]["line"] + 1, r["range"]["start"]["character"] + 1) for r in refs))
edits = list((edit.get("changes") or {}).get(uri, []))
for dc in edit.get("documentChanges") or []:
    if dc.get("textDocument", {}).get("uri") == uri:
        edits += dc["edits"]
print("rename edits:", sorted((e["range"]["start"]["line"] + 1, e["range"]["start"]["character"] + 1) for e in edits))
if out:
    offs = [0]
    for l in lines:
        offs.append(offs[-1] + len(l))
    o = lambda q: offs[q["line"]] + q["character"]
    for e in sorted(edits, key=lambda e: o(e["range"]["start"]), reverse=True):
        text = text[:o(e["range"]["start"])] + e["newText"] + text[o(e["range"]["end"]):]
    Path(out).write_text(text)
