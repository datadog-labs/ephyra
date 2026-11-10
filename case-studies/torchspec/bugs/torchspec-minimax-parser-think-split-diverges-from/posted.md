**Description**

`MiniMaxParser` (`torchspec/data/parse.py`) formats minimax-m2 conversations by hand on the legacy (`huggingface`) rendering path. Its helper says it follows the template:

> `_extract_thinking`: "Split content into (reasoning, remaining) following MiniMax jinja logic."

The MiniMax-M2.5 `chat_template.jinja` does this for an assistant message:

```
{%- if message.reasoning_content is string %}
    {%- set reasoning_content = message.reasoning_content %}
{%- else %}
    {%- if '</think>' in content %}
        {%- set reasoning_content = content.split('</think>')[0].strip('\n').split('<think>')[-1].strip('\n') %}
        {%- set content = content.split('</think>')[-1].strip('\n') %}
```

The parser differs in two ways:

1. `format()` uses `if not reasoning:`, so `reasoning_content: ""` counts as missing and the content is still split at `</think>`. The template treats any string, including `""`, as the reasoning and keeps the content unchanged.
2. `_extract_thinking` uses `content.split("</think>", 1)` and keeps everything after the first `</think>`. The template keeps only the part after the last `</think>`.

When this happens, the parser's training text differs from what the model's own template produces, and no error is raised. In an earlier assistant turn (before the last user message), case 1 drops the text before `</think>`. For example, `x</think>y` becomes `y`.

This is low priority: minimax-m2 uses the SMG renderer by default (`rendering_backend: auto`), which renders the real template, so only the explicit `huggingface` backend and the modes SMG does not handle reach this parser, and only for turns with an empty `reasoning_content` or more than one `</think>`.

**Steps to reproduce**

`poc.py` (CPU only; downloads only the tokenizer files of `MiniMaxAI/MiniMax-M2.5` at the revision pinned in `tests/tokenizer_contract/fixtures/manifest.json`):

```python
"""MiniMaxParser.format (minimax-m2, legacy "huggingface" backend) does not render
assistant turns the way MiniMax-M2.5's chat_template.jinja does when the content
contains "</think>".

Usage: python poc.py [tokenizer-dir]
Without an argument, downloads the tokenizer files of MiniMaxAI/MiniMax-M2.5 at the revision
pinned in tests/tokenizer_contract/fixtures/manifest.json. CPU only.
Exit 1 if any parser text differs from the chat template text, 0 otherwise.
"""
import sys

from transformers import AutoTokenizer
from torchspec.data.parse import create_parser
from torchspec.data.template import TEMPLATE_REGISTRY

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

tok = AutoTokenizer.from_pretrained(path)
parser = create_parser(tok, TEMPLATE_REGISTRY.get("minimax-m2"))

cases = {
    # reasoning_content is an explicit empty string
    "empty reasoning_content": {"content": "x</think>y", "reasoning_content": ""},
    # two </think> markers, no reasoning_content
    "two </think>": {"content": "a</think>b</think>c"},
}
bad = False
for name, assistant in cases.items():
    messages = [{"role": "user", "content": "hi"}, {"role": "assistant", **assistant}]
    template = tok.apply_chat_template(messages, tokenize=False)
    torchspec = parser.format(messages)
    turn = lambda s: s.split("]~b]ai\n", 1)[1].split("[e~[", 1)[0]
    ok = template == torchspec
    bad |= not ok
    print(f"{name}: template={turn(template)!r} torchspec={turn(torchspec)!r} "
          f"{'ok' if ok else 'DIFFERENT'}")
sys.exit(1 if bad else 0)
```

Command:

```
python poc.py
```

**Observed and expected output**

```
python poc.py
# template:  empty reasoning_content: 'x</think>y'
# torchspec: empty reasoning_content: '<think>\nx\n</think>\n\ny'
# template:  two </think>: '<think>\na\n</think>\n\nc'
# torchspec: two </think>: '<think>\na\n</think>\n\nb</think>c'
```

Actual stdout (exit code 1):

```
empty reasoning_content: template='x</think>y' torchspec='<think>\nx\n</think>\n\ny' DIFFERENT
two </think>: template='<think>\na\n</think>\n\nc' torchspec='<think>\na\n</think>\n\nb</think>c' DIFFERENT
```

Expected: both lines `ok`, with `parser.format(messages)` equal to `tokenizer.apply_chat_template(messages, tokenize=False)`.

Same bug in an earlier turn. For `[user "hi", assistant {content: "x</think>y", reasoning_content: ""}, user "ok", assistant "z"]`, the first assistant turn renders as:

```
# template:  ]~b]ai\nx</think>y[e~[
# torchspec: ]~b]ai\ny[e~[
```

**When it applies**

With the default `dataset.rendering_backend: auto`, minimax-m2 text rows go to SMG, which renders with the real template (`docs/smg-tokenization.md`). The parser runs with `rendering_backend: huggingface`, and in the modes that `auto` sends to the existing path (preformatted text, generation prefix, and non-K3 deferred/decode training).

**Environment**

- TorchSpec commit c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9 (main, 2026-10-29)
- torch 2.15.0, transformers 5.20.2, tokenizers 0.23.3, huggingface_hub 2.4.0, jinja2 3.1.6
- Python 3.12.3, Linux 6.8 x86_64 (glibc 2.39)
- CPU only, no GPU

**Latest release**

The repository has no tagged release. `MiniMaxParser` was added in #45 (commit 2c25d5d). This code has not changed since, and current main has the bug.

**Related issues and PRs**

I searched the open and closed issues and PRs for MiniMax, think, reasoning_content, parser, chat template and legacy. None of them reports this bug. Related: #45, which added `MiniMaxParser`, and #205, a different MiniMax bug in the SMG renderer.

A fix with a test is available.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
