import sys

import torch
from huggingface_hub import snapshot_download
from transformers import AutoTokenizer
from torchspec.data.renderers.kimi_k3 import K3Renderer
from torchspec.models.ops.loss_mask import compute_assistant_loss_mask

path = snapshot_download(
    "moonshotai/Kimi-K3",
    revision="9f62e4e9fffbd0a83ddd60e1c209d828994b3569",
    allow_patterns=["config.json", "configuration_kimi_k3.py", "encoding_k3.py", "generation_config.json",
                    "tiktoken.model", "tokenization_kimi.py", "tokenizer_config.json"],
)
tok = AutoTokenizer.from_pretrained(path, trust_remote_code=True)
renderer = K3Renderer(tok)

messages = [
    {"role": "user", "content": "hi"},
    {"role": "assistant", "content": "hello", "name": "x"},
]
ids, renderer_mask = renderer.render(messages, max_seq_length=4096)
header, end, skip = renderer.get_assistant_token_ids()
matcher_mask = compute_assistant_loss_mask(torch.tensor(ids), header, end, skip_after_header=skip).tolist()

supervised = lambda mask: tok.decode([t for t, m in zip(ids, mask) if m])
print("rendered:      ", repr(tok.decode(ids)))
print("renderer mask: ", repr(supervised(renderer_mask)))
print("matcher mask:  ", repr(supervised(matcher_mask)))
sys.exit(0 if matcher_mask == renderer_mask else 1)
