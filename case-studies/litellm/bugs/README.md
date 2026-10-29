# Bugs: litellm

The bugs the case studies found in litellm. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [Gemini conversion drops a parallel tool call when two calls have the same name and arguments, leaving more functionResponse than functionCall parts](litellm-gemini-drops-identical-parallel-tool-cal/) | open | [link](https://github.com/BerriAI/litellm/issues/48831) |
| [Gemini streaming reports finish_reason "length" for a tool call that ends with MAX_TOKENS, non-streaming reports "tool_calls"](litellm-gemini-stream-tool-call-max-tokens-finis/) | open | [link](https://github.com/BerriAI/litellm/issues/48715) |
| [OpenAI-compatible streaming keeps finish_reason "stop" when the response has tool calls, non-streaming reports "tool_calls"](litellm-openai-stream-keeps-stop-with-tool-calls/) | open | [link](https://github.com/BerriAI/litellm/issues/48716) |
| [responses() sends a function_call_output of empty text parts to the provider as the JSON text of the list](litellm-responses-empty-tool-output-list-sent-as/) | open | [link](https://github.com/BerriAI/litellm/issues/48832) |
| [responses() with Gemini fails with "Missing corresponding tool call" when a reasoning item sits between two function_call items](litellm-responses-reasoning-between-function-cal/) | open | [link](https://github.com/BerriAI/litellm/issues/49063) |
