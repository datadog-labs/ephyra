# PoC: AsyncPutManager raises one async put failure twice

Run it from a TorchSpec checkout installed in the current Python environment (CPU only, no GPU, no Mooncake needed):

    python poc.py

On c4fc3e0 it prints:

    wait_for_buffer raised: async batch_put_from failed: sample0_hs (code=-200)
    check_last_error raised: async batch_put_from failed: sample0_hs (code=-200)

The expected second line is `check_last_error: nothing raised`.

`repro.sh <build-dir>` runs `check_repro.py` (the same check, with exit codes) using `<build-dir>/venv/bin/python`. Exit 1 means it reproduces, 0 means it does not.
