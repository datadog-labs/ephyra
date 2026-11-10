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
