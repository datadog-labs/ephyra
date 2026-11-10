# Repro: DFlashConfig rejects a Llama-3.1-style rope_theta + llama3 rope_scaling

CPU only. No downloads or tokenizer files are needed.

    pip install -e <TorchSpec checkout>   # with transformers 5.x (tested: 5.20.2)
    python poc.py

Expected: "LlamaConfig: ok" then "DFlashConfig: ok".
Actual on main c4fc3e0: "LlamaConfig: ok", then
KeyError: "Missing required keys in `rope_parameters` for 'rope_type'='llama3': {'rope_theta'}".

Each config gets its own copy of the dict on purpose. LlamaConfig adds rope_theta to the dict
passed to it. Reusing that dict hides this KeyError and shows the separate AttributeError
(missing max_position_embeddings) instead.
