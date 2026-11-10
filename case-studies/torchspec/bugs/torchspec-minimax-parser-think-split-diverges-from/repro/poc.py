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
