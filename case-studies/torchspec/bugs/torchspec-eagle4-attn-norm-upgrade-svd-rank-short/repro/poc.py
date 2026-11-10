# EAGLE3 -> EAGLE-4 (attn_norm) warm start, as in tests/test_initial_draft_model_path.py
# test_upgrades_a_pruned_eagle3_draft_to_eagle4, but head_dim 8 instead of 16:
# stacked q/k/v = (2 + 1 + 1) * 8 = 32 rows < hidden_size 64.
import sys, tempfile, torch
from safetensors.torch import save_file
from torchspec.models.draft.auto import AutoDraftModelConfig, AutoEagle3DraftModel
from torchspec.models.draft.keymap import to_export_keys
from torchspec.training.checkpoint import load_initial_draft_weights

cfg = dict(architectures=["LlamaForCausalLMEagle3"], model_type="llama", hidden_size=64,
           intermediate_size=128, num_hidden_layers=1, num_attention_heads=2, num_key_value_heads=1,
           head_dim=8, vocab_size=128, draft_vocab_size=128, max_position_embeddings=256,
           rms_norm_eps=1e-6, rope_theta=10000.0)
build = lambda c: AutoEagle3DraftModel.from_config(AutoDraftModelConfig.from_dict(c), torch_dtype=torch.float32)

eagle3 = build(cfg)
eagle4 = build({**cfg, "architectures": ["LlamaForCausalLMEagle4"], "attn_norm": True})
with tempfile.TemporaryDirectory() as d:
    save_file(to_export_keys(eagle3.state_dict()), f"{d}/model.safetensors")
    try:
        load_initial_draft_weights(eagle4, d)
    except RuntimeError as e:
        print("FAILED:", e); sys.exit(1)
print("loaded, eh_proj", tuple(eagle4.state_dict()["midlayer.eh_proj.weight"].shape))
