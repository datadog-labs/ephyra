# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""Runs generated programs, each in a forked child, and prints one JSON line per program:
{"file", "status": "ok"|"exc"|"timeout"|"crash", "error", "obs": {k: [types]}}."""
import json
import os
import runpy
import signal
import sys
import traceback

TIMEOUT = float(os.environ.get("TYZ_PY_TIMEOUT", "3"))


def run_one(path: str) -> dict:
    r, w = os.pipe()
    pid = os.fork()
    if pid == 0:
        os.close(r)
        signal.alarm(int(TIMEOUT) + 1)
        sys.setrecursionlimit(2000)
        out: dict = {"file": path}
        try:
            import _rt
            _rt.OBS.clear()
            devnull = open(os.devnull, "w")
            sys.stdout = devnull
            runpy.run_path(path, run_name="__main__")
            out["status"] = "ok"
        except BaseException as e:  # noqa: BLE001
            out["status"] = "exc"
            out["error"] = f"{type(e).__name__}: {e}"[:300]
            out["tb"] = traceback.format_exc()[-1200:]
        import _rt
        out["obs"] = {str(k): sorted(v) for k, v in _rt.OBS.items()}
        os.write(w, json.dumps(out).encode())
        os.close(w)
        os._exit(0)
    os.close(w)
    chunks = []
    while True:
        b = os.read(r, 1 << 16)
        if not b:
            break
        chunks.append(b)
    os.close(r)
    _, status = os.waitpid(pid, 0)
    if not chunks:
        return {"file": path, "status": "crash" if not os.WIFSIGNALED(status) or os.WTERMSIG(status) != signal.SIGALRM else "timeout", "obs": {}}
    return json.loads(b"".join(chunks))


def main() -> None:
    d = sys.argv[1]
    sys.path.insert(0, d)
    for name in sys.argv[2:]:
        print(json.dumps(run_one(os.path.join(d, name))), flush=True)


if __name__ == "__main__":
    main()
