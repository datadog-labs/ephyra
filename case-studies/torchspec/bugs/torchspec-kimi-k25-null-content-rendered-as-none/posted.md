**Description**

`KimiK25Parser.format` in `torchspec/data/parse.py` (template `kimi-k25-vlm`) handles non-string content like this:

```python
content = msg.get("content", "")
if not isinstance(content, str):
    if isinstance(content, list):
        ...
    else:
        content = str(content)
```

When a message has `"content": None`, `str(None)` makes it the four characters `None`. This is common in OpenAI-format tool-use data, where an assistant turn that only makes tool calls has `content: null`. The assistant turn then becomes `<think></think>None<|tool_calls_section_begin|>...`. Because it is inside the assistant span, `parse()` puts `None` into the loss mask. The draft model is trained to emit "None" before its tool calls. There is no error or warning.

Expected: a null content renders as nothing, the same as the model's own chat template. The class docstring says it is a "Parser for Kimi-K2.5 model with manual string formatting", in place of that template. Kimi-K2.5's `chat_template.jinja` (moonshotai/Kimi-K2.5 @ 4d01dfe0) renders content like this:

```jinja
{%- set c = msg.get('content') -%}
{%- if c is string -%}
  {{ c }}
{%- elif c is not none -%}
```

The template outputs nothing for `None`. The sibling `MiniMaxParser.format` in the same file already does `content = msg.get("content", "") or ""`.

**Steps to reproduce**

You need a TorchSpec checkout on the Python path, CPU only. The script downloads the Kimi-K2.5 tokenizer files at a pinned revision (`trust_remote_code` is needed for `tokenization_kimi.py`). Save it as `poc.py`:

```python
import sys
from huggingface_hub import snapshot_download
from transformers import AutoTokenizer
from torchspec.data.parse import create_parser
from torchspec.data.template import TEMPLATE_REGISTRY

path = snapshot_download("moonshotai/Kimi-K2.5", revision="4d01dfe0332d63057c186e0b262165819efb6611",
                         allow_patterns=["chat_template.jinja", "config.json", "tokenizer_config.json",
                                         "tiktoken.model", "*.py"])
tok = AutoTokenizer.from_pretrained(path, trust_remote_code=True)
parser = create_parser(tok, TEMPLATE_REGISTRY.get("kimi-k25-vlm"))

messages = [{"role": "user", "content": "hi"}, {"role": "assistant", "content": None}]

first_turn = lambda s: s.split("<|im_assistant|>assistant<|im_middle|>")[1].split("<|im_end|>")[0]
template = first_turn(tok.apply_chat_template(messages, tokenize=False))
torchspec = first_turn(parser.format(messages))
ids, mask = parser.parse(messages, max_length=64)
print("chat template assistant turn:", repr(template))
print("torchspec assistant turn:    ", repr(torchspec))
print("torchspec supervised text:   ", repr(tok.decode(ids[mask.bool()].tolist())))
sys.exit(1 if template != torchspec else 0)
```

```
python poc.py
# chat template (Kimi-K2.5 jinja): assistant turn '<think></think>'
# torchspec (KimiK25Parser):       assistant turn '<think></think>None', and 'None' is supervised
```

**Observed output**

```
chat template assistant turn: '<think></think>'
torchspec assistant turn:     '<think></think>None'
torchspec supervised text:    '<think></think>None'
```

**Expected output**

```
chat template assistant turn: '<think></think>'
torchspec assistant turn:     '<think></think>'
torchspec supervised text:    '<think></think>'
```

The same happens to a tool-call turn. With `{"role": "assistant", "content": None, "tool_calls": [{"id": "functions.f:0", "type": "function", "function": {"name": "f", "arguments": "{}"}}]}` the outputs are:

```
# chat template: <|im_assistant|>assistant<|im_middle|><think></think><|tool_calls_section_begin|><|tool_call_begin|>functions.f:0...
# torchspec:     <|im_assistant|>assistant<|im_middle|><think></think>None<|tool_calls_section_begin|><|tool_call_begin|>functions.f:0...
```

User, system and tool messages with `content: None` are rendered as `None` in the same way. They are not supervised, but they still change the training text.

**Environment**

- TorchSpec commit c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9 (current main, re-checked 2026-11-02)
- torch 2.15.0+cu132, transformers 5.20.2, tokenizers 0.23.3
- Python 3.12.3, Linux x86_64 (Ubuntu, glibc 2.39)
- CPU only, no GPU

**Latest release**

The repository has no tagged releases (version.txt says 0.1.0). The `content = str(content)` branch has been in `KimiK25Parser.format` since the initial commit (4643a09), so every version has it.

**Related issues and PRs**

- #115 "fix(kimi-k25): preserve thinking content in offline data formatting" and #108 "Fix Kimi generation prompt thinking block" fixed other Kimi-K2.5 formatting differences. Neither one covers null content.
- I found no existing issue or PR about null content. I searched the titles of all open and closed issues and PRs.

**Fix**

A one-line fix is available: map `None` to `""` in `KimiK25Parser.format` before the type check. It does not include a new test yet.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
