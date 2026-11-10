**Description**

With the `kimi-k25-vlm` template, `KimiK25Parser._format_tool_calls` (torchspec/data/parse.py:441-453) puts `function.arguments` into the text with an f-string. Arguments are often an object, as in OpenAI-style data. When they are, the training text gets the Python repr `{'q': 'a', 'n': True}`: single quotes and `True`/`False`/`None`. The text is supposed to contain JSON.

The parser is documented as reproducing the Kimi-K2.5 chat template by hand (docstring: "Parser for Kimi-K2.5 model with manual string formatting"). That template serializes non-string arguments as JSON (moonshotai/Kimi-K2.5 `chat_template.jinja`, line 34):

```
{% if tool_call['function']['arguments'] is string %}{{ tool_call['function']['arguments'] }}{% else %}{{ tool_call['function']['arguments'] | tojson }}{% endif %}
```

The repr text is inside the loss mask, so the draft model is trained on a tool-call syntax the target model never produces. In the same file, `MiniMaxParser._format_tool_calls` already handles dict arguments with `json.dumps` (parse.py:593-601). String arguments are not affected.

**Steps to reproduce**

`poc.py` (CPU only; downloads only the tokenizer and template files of moonshotai/Kimi-K2.5 at a pinned revision):

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

call = {"id": "call1", "type": "function", "function": {"name": "search", "arguments": {"q": "a", "n": True}}}
messages = [
    {"role": "user", "content": "hi"},
    {"role": "assistant", "content": "", "tool_calls": [call]},
    {"role": "tool", "tool_call_id": "call1", "content": "r"},
    {"role": "assistant", "content": "ok"},
]
args = lambda s: s.split("<|tool_call_argument_begin|>")[1].split("<|tool_call_end|>")[0]
print("chat template arguments:", args(tok.apply_chat_template(messages, tokenize=False)))
print("torchspec arguments:    ", args(parser.format(messages)))
```

```
python poc.py
# chat template (expected): chat template arguments: {"q": "a", "n": true}
# torchspec (actual):       torchspec arguments:     {'q': 'a', 'n': True}
```

The supervised tokens from `parser.parse(messages, max_length=256)` contain the repr too:

```
tok.decode(ids[mask.bool()].tolist())
# torchspec: "<think></think><|tool_calls_section_begin|><|tool_call_begin|>call1<|tool_call_argument_begin|>{'q': 'a', 'n': True}<|tool_call_end|><|tool_calls_section_end|><think></think>ok"
```

Expected: the argument text matches the chat template, `{"q": "a", "n": true}` (the `tojson` output).

**Environment**

- TorchSpec main at c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9 (latest as of 2026-11-02)
- torch 2.15.0, transformers 5.20.2, tokenizers 0.23.3
- Python 3.12.3, Linux 6.8 x86_64 (glibc 2.39)
- CPU only, no GPU

**Latest release**

TorchSpec has no releases. The f-string has been in the code since the initial commit (4643a09), so every version of main that has KimiK25Parser behaves this way.

**Related issues and PRs**

I found no existing issue or PR, open or closed, about this. #204 is about tool-call arguments too, but it covers integer rounding in the SMG binding, which is a different code path.

A fix with a test is available.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
