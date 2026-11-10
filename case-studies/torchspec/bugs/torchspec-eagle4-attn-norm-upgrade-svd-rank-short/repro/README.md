# EAGLE3 -> EAGLE-4 attn_norm warm start rejects its own upgrade

Run from a TorchSpec checkout (or any env with torchspec installed); CPU only, no tokenizer, no network:

    python poc.py
    # expected: loaded, eh_proj (64, 128)        (exit 0)
    # trunk c4fc3e0: FAILED: Incompatible initial LlamaForCausalLMEagle3 checkpoint: ...
    #   shape_mismatch=['midlayer.eh_proj.weight', 'midlayer.self_attn.k_proj.weight', ...]   (exit 1)

Exit 1 = reproduces, 0 = fixed.
