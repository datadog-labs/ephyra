# responses-empty-tool-output-list-sent-as PoC

Runs offline: poc.py starts a local mock of the Anthropic /v1/messages endpoint and records the request litellm sends.

    pip install litellm mcp      # litellm 1.106.5 needs `mcp` for responses()
    LITELLM_LOCAL_MODEL_COST_MAP=True python3 poc.py
    # or, against a harness build:  ./repro.sh <build-dir>

Exit 1 = bug present (the tool_result for output [{"type":"input_text","text":""}] differs from output ""),
0 = fixed, 2 = setup error.
