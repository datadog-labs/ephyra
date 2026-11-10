import sys

from huggingface_hub import snapshot_download
from transformers import AutoTokenizer

from torchspec.models.ops.loss_mask import compute_assistant_loss_mask

path = snapshot_download(
    "Qwen/Qwen3-0.6B",
    revision="c1899de289a04d12100db370d81485cdf75e47ca",
    allow_patterns=["config.json", "tokenizer.json", "tokenizer_config.json", "vocab.json", "merges.txt"],
)
tok = AutoTokenizer.from_pretrained(path)

# Same matcher as torchspec.utils.processing.get_assistant_token_ids for qwen templates.
header = tok.encode("<|im_start|>assistant", add_special_tokens=False)
end = tok.encode("<|im_end|>", add_special_tokens=False)

text = (
    "<|im_start|>user\nhi<|im_end|>\n"
    "<|im_start|>assistant\nOLD ANSWER<|im_end|>\n"
    "<|im_start|>user\nagain<|im_end|>\n"
    "<|im_start|>assistant\n<|im_end|>\n"  # last assistant turn is empty
)
ids = tok.encode(text, add_special_tokens=False)
mask = compute_assistant_loss_mask(ids, header, end, last_turn_only=True, skip_after_header=1)
masked = tok.decode([t for t, m in zip(ids, mask.tolist()) if m])
print(f"last_turn_only=True trains on: {masked!r}")
sys.exit(1 if masked else 0)
