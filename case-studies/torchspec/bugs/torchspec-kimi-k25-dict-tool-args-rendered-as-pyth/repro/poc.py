"""KimiK25Parser (template kimi-k25-vlm) writes dict tool-call arguments as a Python repr
({'q': 'a'}) instead of JSON ({"q": "a"}) as the Kimi-K2.5 chat template does.

Usage: python poc.py [tokenizer-dir]
Without an argument, downloads the Kimi-K2.5 tokenizer files at a pinned revision. CPU only.
Exit 1 if the parser's tool-call arguments differ from the chat template's, 0 otherwise.
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
        "moonshotai/Kimi-K2.5",
        revision="4d01dfe0332d63057c186e0b262165819efb6611",
        allow_patterns=["chat_template.jinja", "config.json", "tokenizer_config.json",
                        "tiktoken.model", "*.py"],
    )

tok = AutoTokenizer.from_pretrained(path, trust_remote_code=True)
parser = create_parser(tok, TEMPLATE_REGISTRY.get("kimi-k25-vlm"))

# OpenAI-style tool call whose arguments are an object, not a JSON string.
call = {"id": "call1", "type": "function", "function": {"name": "search", "arguments": {"q": "a", "n": True}}}
messages = [
    {"role": "user", "content": "hi"},
    {"role": "assistant", "content": "", "tool_calls": [call]},
    {"role": "tool", "tool_call_id": "call1", "content": "r"},
    {"role": "assistant", "content": "ok"},
]

args = lambda s: s.split("<|tool_call_argument_begin|>")[1].split("<|tool_call_end|>")[0]
template = args(tok.apply_chat_template(messages, tokenize=False))
torchspec = args(parser.format(messages))
print("chat template arguments:", template)
print("torchspec arguments:    ", torchspec)
sys.exit(1 if template != torchspec else 0)
