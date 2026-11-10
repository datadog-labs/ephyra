# yarn-rope-missing-optional-keys-pass-non: PoC

Needs a TorchSpec checkout installed (`pip install -e .`) with torch and transformers. CPU only.
No tokenizer files or downloads are needed.

    python poc.py

On TorchSpec c4fc3e0 it prints `transformers: ok`, then raises
`TypeError: unsupported operand type(s) for *: 'NoneType' and 'int'`.
With the fix applied it prints `transformers: ok` and `torchspec: ok`.

Exit status: 0 when the model builds, 1 (an uncaught exception) when the bug reproduces.
