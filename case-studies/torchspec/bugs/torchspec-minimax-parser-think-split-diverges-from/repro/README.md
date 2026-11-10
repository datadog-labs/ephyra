# PoC: MiniMaxParser.format vs MiniMax-M2.5 chat template

Needs a TorchSpec checkout installed in a Python env (torch, transformers, huggingface_hub). CPU only.

    python poc.py              # downloads tokenizer files of MiniMaxAI/MiniMax-M2.5 @ 2515a3ea0afe
    python poc.py <tok-dir>    # or use a local directory with chat_template.jinja and tokenizer files

Exit 1 = parser text differs from the template text (bug present), 0 = identical.

Checked on TorchSpec c4fc3e0 (main, 2026-10-29): exit 1. With the candidate fix: exit 0.
