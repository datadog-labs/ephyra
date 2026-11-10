# Repro: KimiK25Parser renders null content as "None"

Run from a TorchSpec checkout (or with it on PYTHONPATH), CPU only:

    pip install -e .            # or: export PYTHONPATH=/path/to/TorchSpec
    python poc.py               # downloads Kimi-K2.5 tokenizer files at revision 4d01dfe0332d63057c186e0b262165819efb6611
    python poc.py <tok-dir>     # or use a local copy of those files

The tokenizer needs `trust_remote_code=True` (tokenization_kimi.py), and HF_HOME must be writable.
Exit 1 = the bug reproduces (the parser writes "None" into the assistant turn). Exit 0 = output matches the chat template.

Checked on 2026-11-02: exit 1 on upstream main c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9. Exit 0 with the
fix (map None to "" in KimiK25Parser.format, patch 012).
Re-checked 2026-11-02 in the report phase: same results.
