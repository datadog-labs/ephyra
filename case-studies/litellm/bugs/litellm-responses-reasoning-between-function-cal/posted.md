# [Bug]: responses() with Gemini fails with "Missing corresponding tool call" when a reasoning item sits between two function_call items

Posted as https://github.com/BerriAI/litellm/issues/49063 on 2026-10-29. The text below is the issue as posted.

---

### Description

With `litellm.responses()` on a Gemini model, a conversation where a reasoning item with text sits between two `function_call` items fails before anything is sent, with `APIConnectionError: Missing corresponding tool call for tool response message` for the first call's result. This is the shape Gemini itself streams when it thinks between two function calls (`[functionCall, thought, functionCall]`), so feeding a turn's output back with both tool results, as OpenAI's function-calling guide does, breaks the next request

I'd expect the request to go through with both calls and both results. Empty or encrypted-only reasoning items work, and the same history works with Anthropic and Bedrock models. It also reproduces on the head of open PR #47928. #22578 (closed) had the same error for plain parallel calls, which now work on main

### Config

```yaml
# SDK only, no config.yaml
litellm.responses(model="gemini/gemini-2.5-flash", input=[...], api_key="x")
```

### LiteLLM Version

v1.106.5, also main at 471918c3

### Steps to Repro

The error is raised while litellm builds the Gemini request, so no server is needed

```python
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
```

Output on v1.106.5 and on main 471918c3 (truncated)

```
APIConnectionError litellm.APIConnectionError: Missing corresponding tool call for tool response message. Received - message={'role': 'tool', 'content': 'sunny', 'tool_call_id': 'call_a'}, last_message_with_tool_calls={'role': 'assistant', 'reasoning_content': 'Now Rome too.', 'tool_calls': [{'id': ...
```

### Which part of LiteLLM is this about?

LLM translation: a specific provider's request or response

### How are you deploying?

pip / Python SDK

