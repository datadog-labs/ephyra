**Description**

The minimax-m2 end-of-turn marker is `[e~[`. It starts and ends with `[`. If an assistant message ends in `[e~`, the rendered turn is `...[e~[e~[`. `SmgRenderer.render` finds the end of the turn with `text.find(self.end, body)` (`torchspec/data/renderers/smg.py`), and that returns the first match. The first match starts inside the content, three characters too early. The real marker is never found.

As a result the loss mask is wrong, and no error is raised:
- With `smg_include_end_token=True` (the default), the content's `[e~` plus the first `[` of the marker are supervised as the "end marker". The rest of the real marker (`e~[`) gets mask 0.
- With `smg_include_end_token=False`, the trailing `[e~` of the content gets mask 0.

The literal-delimiter check earlier in `render()` does not catch this, because the content does not contain the whole marker `[e~[`.

`docs/smg-tokenization.md` says:

> `smg_include_end_token` defaults to true; the formatting newline after the end marker is not supervised. [...] Ambiguous assistant boundaries and discarded supervised reasoning/tool calls are rejected.

The comment above the delimiter check in `smg.py` says:

> Delimiter searching is only safe when payloads cannot forge those delimiters. Fail instead of training user/tool text as assistant text.

and the `render()` docstring lists "payloads forge delimiters" under `Raises: ValueError`. Here the payload forges an earlier end-marker match. Expected: either the mask covers the full content plus the real marker, or `render()` raises `ValueError` because the boundary is ambiguous. Actual: a silently shifted mask.

Among the registry profiles only minimax-m2 is affected, because only its marker overlaps itself (`<|im_end|>` and `<|eot_id|>` do not). The SMG backend is the default (`rendering_backend: auto`) for minimax-m2.

**Steps to reproduce**

`poc.py` (CPU only; downloads only the tokenizer files of `MiniMaxAI/MiniMax-M2.5` at the revision pinned in `tests/tokenizer_contract/fixtures/manifest.json`):

```python
"""SmgRenderer (minimax-m2) puts the assistant span end inside the content when the
content ends in "[e~", a prefix of the end marker "[e~[".

Usage: python poc.py [tokenizer-dir]
Without an argument, downloads the tokenizer files of MiniMaxAI/MiniMax-M2.5 at the revision
pinned in tests/tokenizer_contract/fixtures/manifest.json. CPU only.
Exit 1 if the supervised text is wrong, 0 if it is right or render raises ValueError.
"""
import sys

from transformers import AutoTokenizer
from torchspec.data.renderers.smg import SmgRenderer

if len(sys.argv) > 1:
    path = sys.argv[1]
else:
    from huggingface_hub import snapshot_download

    path = snapshot_download(
        "MiniMaxAI/MiniMax-M2.5",
        revision="2515a3ea0afec1dd22d0c7794b3a91557804d195",
        allow_patterns=["chat_template.jinja", "config.json", "tokenizer.json",
                        "tokenizer_config.json", "vocab.json", "merges.txt"],
    )

hf = AutoTokenizer.from_pretrained(path)
bad = False
for content in ["ok", "ok [e~"]:
    for include_end in [True, False]:
        renderer = SmgRenderer(path, "minimax-m2", include_end_token=include_end)
        messages = [{"role": "user", "content": "hi"}, {"role": "assistant", "content": content}]
        expected = content + ("[e~[" if include_end else "")
        try:
            ids, mask = renderer.render(messages, max_seq_length=4096)
        except ValueError as exc:  # rejecting the ambiguous row is acceptable
            print(f"content={content!r} include_end={include_end}: ValueError: {exc}")
            continue
        got = hf.decode([i for i, m in zip(ids, mask) if m])
        ok = got == expected
        bad |= not ok
        print(f"content={content!r} include_end={include_end}: supervised={got!r} "
              f"expected={expected!r} {'ok' if ok else 'WRONG'}")
sys.exit(1 if bad else 0)
```

Command:

```
python poc.py
```

**Observed and expected output**

The rendered text for the second row is the same in SMG and in `transformers` `apply_chat_template` (tail shown, `\n` is a newline):

```
...]~b]user\nhi[e~[\n]~b]ai\nok [e~[e~[\n
```

Output of `python poc.py` (exit status 1):

```
python poc.py
# expected: the content, plus the full end marker "[e~[" when include_end=True
content='ok' include_end=True: supervised='ok[e~[' expected='ok[e~[' ok
content='ok' include_end=False: supervised='ok' expected='ok' ok
# torchspec SmgRenderer (actual):
content='ok [e~' include_end=True: supervised='ok [e~[' expected='ok [e~[e~[' WRONG
content='ok [e~' include_end=False: supervised='ok ' expected='ok [e~' WRONG
```

`supervised` is the decoded text of the tokens with mask 1. Raising `ValueError` for the `ok [e~` rows would also be correct; the script then prints the error and exits 0.

**Environment**

- TorchSpec commit c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9 (main, 2026-10-29), SMG binding built from `bindings/smg-tokenizer` with `pip install -e .`
- torch 2.15.0, transformers 5.20.2, tokenizers 0.23.3, huggingface_hub 2.4.0
- Python 3.12.3, Linux 6.8 x86_64 (glibc 2.39)
- CPU only, no GPU

**Latest release**

The repository has no tagged release. `smg.py` was added in #201 (commit 4094382), and every commit since then has the bug, including current main.

**Related issues and PRs**

I searched open and closed issues and PRs for MiniMax, SMG, delimiter, end marker, end token, loss mask, overlap and span. Nothing matches. Related: #201, which added the SMG renderer.

A fix with a test is available.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
