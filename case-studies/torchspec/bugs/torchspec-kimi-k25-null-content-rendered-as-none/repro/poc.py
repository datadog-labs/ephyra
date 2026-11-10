"""KimiK25Parser (template kimi-k25-vlm) renders a null message content as the text "None"
and supervises it.

Usage: python poc.py [tokenizer-dir]
Without an argument, downloads the Kimi-K2.5 tokenizer files at a pinned revision. CPU only.
Exit 1 if the parser writes "None" into the assistant turn, 0 otherwise.
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

# An assistant turn with null content, as in an OpenAI-style turn that only has tool_calls.
messages = [{"role": "user", "content": "hi"}, {"role": "assistant", "content": None}]

first_turn = lambda s: s.split("<|im_assistant|>assistant<|im_middle|>")[1].split("<|im_end|>")[0]
template = first_turn(tok.apply_chat_template(messages, tokenize=False))
torchspec = first_turn(parser.format(messages))
ids, mask = parser.parse(messages, max_length=64)
print("chat template assistant turn:", repr(template))
print("torchspec assistant turn:    ", repr(torchspec))
print("torchspec supervised text:   ", repr(tok.decode(ids[mask.bool()].tolist())))
sys.exit(1 if template != torchspec else 0)
