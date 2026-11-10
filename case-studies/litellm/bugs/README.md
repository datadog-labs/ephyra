# Bugs: litellm

The bugs the case studies found in litellm. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [[Bug]: anthropic.messages.acreate(stream=True) on Bedrock raises the raw BedrockError for an exception event in the stream](litellm-bedrock-messages-stream-midstream-error-/) | open | [link](https://github.com/BerriAI/litellm/issues/49935) |
| [[Bug]: A cache hit returns finish_reason "tool_calls" for a stored Anthropic response that had "stop"](litellm-cache-hit-rewrites-stop-to-tool-calls/) | open | [link](https://github.com/BerriAI/litellm/issues/49929) |
| [Gemini conversion drops a parallel tool call when two calls have the same name and arguments, leaving more functionResponse than functionCall parts](litellm-gemini-drops-identical-parallel-tool-cal/) | open | [link](https://github.com/BerriAI/litellm/issues/48831) |
| [Gemini streaming reports finish_reason "length" for a tool call that ends with MAX_TOKENS, non-streaming reports "tool_calls"](litellm-gemini-stream-tool-call-max-tokens-finis/) | open | [link](https://github.com/BerriAI/litellm/issues/48715) |
| [[Bug]: anthropic.messages.acreate(stream=True) caches an empty reply from a non-Anthropic model and replays it on every repeat](litellm-messages-stream-cache-stores-empty-text-/) | fixed | [link](https://github.com/BerriAI/litellm/issues/49931) |
| [[Bug]: OpenAI HTTP 403 raises APIError or BadRequestError instead of PermissionDeniedError](litellm-openai-403-not-mapped-to-permission-deni/) | open | [link](https://github.com/BerriAI/litellm/issues/49936) |
| [[Bug]: responses() sends temperature to OpenAI o-series models even with drop_params=True](litellm-openai-responses-o-series-forwards-tempe/) | open | [link](https://github.com/BerriAI/litellm/issues/49927) |
| [OpenAI-compatible streaming keeps finish_reason "stop" when the response has tool calls, non-streaming reports "tool_calls"](litellm-openai-stream-keeps-stop-with-tool-calls/) | fixed | [link](https://github.com/BerriAI/litellm/issues/48716) |
| [[Bug]: /engines/{model}/chat/completions and /openai/deployments/{model}/chat/completions return the provider model name without streaming](litellm-proxy-path-model-nonstream-returns-provi/) | open | [link](https://github.com/BerriAI/litellm/issues/49932) |
| [[Bug]: Cached responses() call on an Anthropic model fails with "provider returned a response with no 'choices'"](litellm-responses-bridge-cache-hit-misread-as-ch/) | open | [link](https://github.com/BerriAI/litellm/issues/49925) |
| [[Bug]: max_messages is counted on the translated chat messages for responses() and anthropic.messages on bridged providers](litellm-responses-bridge-cache-ignores-max-messa/) | open | [link](https://github.com/BerriAI/litellm/issues/49928) |
| [responses() sends a function_call_output of empty text parts to the provider as the JSON text of the list](litellm-responses-empty-tool-output-list-sent-as/) | fixed | [link](https://github.com/BerriAI/litellm/issues/48832) |
| [responses() with Gemini fails with "Missing corresponding tool call" when a reasoning item sits between two function_call items](litellm-responses-reasoning-between-function-cal/) | open | [link](https://github.com/BerriAI/litellm/issues/49063) |
| [[Bug]: Router stream=True with a Gemini deployment raises the raw VertexAIError (async) or a TypeError (sync) when the stream fails to open](litellm-router-deferred-stream-open-error-unmapp/) | open | [link](https://github.com/BerriAI/litellm/issues/49933) |
| [[Bug]: Sync Router.responses() ignores the Router's num_retries and fallbacks, Router.aresponses() honours them](litellm-router-sync-responses-ignores-retries-an/) | open | [link](https://github.com/BerriAI/litellm/issues/49938) |
| [[Bug]: A stream that fails after it opens logs the raw provider exception class (e.g. AnthropicError) in error_information](litellm-stream-midstream-failure-logs-unmapped-e/) | open | [link](https://github.com/BerriAI/litellm/issues/49940) |
