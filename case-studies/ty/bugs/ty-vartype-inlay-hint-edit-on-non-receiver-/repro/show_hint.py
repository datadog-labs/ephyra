#!/usr/bin/env python3
"""Ask `ty server` for the inlay hints of a file, print them, apply their text edits
(what an editor does when the user double-clicks a hint) and run `ty check` on the result.

usage: show_hint.py <path-to-ty> <file.py>
exit 1 if `ty check` reports invalid-type-form on the edited source, 0 if it does not.
"""
import json, os, shutil, subprocess, sys, tempfile

ty, src = sys.argv[1], sys.argv[2]
text = open(src).read()
root = tempfile.mkdtemp()
path = os.path.join(root, "repro.py")
shutil.copy(src, path)
uri = "file://" + path

p = subprocess.Popen([ty, "server"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=subprocess.DEVNULL, cwd=root)

def send(msg):
    body = json.dumps(msg).encode()
    p.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    p.stdin.flush()

def recv():
    n = 0
    while True:
        line = p.stdout.readline()
        if not line:
            sys.exit("ty server exited")
        if line in (b"\r\n", b"\n"):
            break
        if line.lower().startswith(b"content-length:"):
            n = int(line.split(b":")[1])
    return json.loads(p.stdout.read(n))

def request(i, method, params):
    send({"jsonrpc": "2.0", "id": i, "method": method, "params": params})
    while True:
        m = recv()
        if m.get("id") == i and "method" not in m:
            return m.get("result")
        if "method" in m and "id" in m:  # server -> client request: answer with null
            send({"jsonrpc": "2.0", "id": m["id"], "result": None})

request(1, "initialize", {
    "processId": None, "rootUri": "file://" + root,
    "workspaceFolders": [{"uri": "file://" + root, "name": "w"}],
    "capabilities": {"textDocument": {"inlayHint": {"dynamicRegistration": False}}},
})
send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
send({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {"textDocument": {
    "uri": uri, "languageId": "python", "version": 1, "text": text}}})
lines = text.split("\n")
hints = request(2, "textDocument/inlayHint", {
    "textDocument": {"uri": uri},
    "range": {"start": {"line": 0, "character": 0}, "end": {"line": len(lines), "character": 0}},
}) or []
request(3, "shutdown", None)
send({"jsonrpc": "2.0", "method": "exit"})

edits = []
for h in hints:
    label = h["label"] if isinstance(h["label"], str) else "".join(x["value"] for x in h["label"])
    pos = h["position"]
    print(f"hint {label!r} at line {pos['line'] + 1}, column {pos['character'] + 1}; "
          f"textEdits {[(e['newText'], e['range']['start']['character'] + 1) for e in h.get('textEdits') or []]}")
    edits += h.get("textEdits") or []

# Apply edits back to front (all edits here are single-line insertions; columns are UTF-16
# units, equal to code points for ASCII sources).
for e in sorted(edits, key=lambda e: (e["range"]["start"]["line"], e["range"]["start"]["character"]), reverse=True):
    s, t = e["range"]["start"], e["range"]["end"]
    assert s["line"] == t["line"]
    l = lines[s["line"]]
    lines[s["line"]] = l[:s["character"]] + e["newText"] + l[t["character"]:]
edited = "\n".join(lines)
print("after accepting the hints:")
print(edited.rstrip("\n"))
open(path, "w").write(edited)
out = subprocess.run([ty, "check", "--python-version", "3.13", "--output-format", "concise",
                      "--no-progress", "repro.py"], cwd=root, capture_output=True, text=True)
print("ty check on the edited file:")
print((out.stdout + out.stderr).rstrip())
sys.exit(1 if "invalid-type-form" in out.stdout else 0)
