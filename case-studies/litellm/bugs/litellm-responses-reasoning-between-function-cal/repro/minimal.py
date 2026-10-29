# No server needed: the error is raised while litellm builds the Gemini request.
import litellm

history = [
    {"role": "user", "content": "Weather in Paris and Rome?"},
    {"type": "function_call", "call_id": "call_a", "name": "get_weather", "arguments": "{}"},
    {"type": "reasoning", "id": "rs_1", "content": [{"type": "output_text", "text": "Now Rome too."}]},
    {"type": "function_call", "call_id": "call_b", "name": "get_weather", "arguments": "{}"},
    {"type": "function_call_output", "call_id": "call_a", "output": "sunny"},
    {"type": "function_call_output", "call_id": "call_b", "output": "sunny"},
]
try:
    litellm.responses(model="gemini/gemini-2.5-flash", input=history, api_key="x",
                      api_base="http://127.0.0.1:9")  # nothing listens here
except Exception as e:
    print(type(e).__name__, str(e).splitlines()[0][:400])
    raise SystemExit(10 if "Missing corresponding tool call" in str(e) else 20)
