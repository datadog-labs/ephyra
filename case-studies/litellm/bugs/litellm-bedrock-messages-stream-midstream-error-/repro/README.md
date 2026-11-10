poc.py starts a local mock of bedrock-runtime on 127.0.0.1 (no network, no AWS account).
Every POST gets HTTP 200 with one AWS event-stream frame: an `internalServerException`
exception event with payload {"message":"boom"}. The script then calls
litellm.anthropic.messages.acreate(stream=True) on a bedrock/ Anthropic model and iterates the stream.

Run: ./repro.sh /path/to/python   (that python needs litellm installed; the 1.106.6 wheel also needs litellm[proxy] extras)
Prints the exception class raised while iterating.
Exit 1: the exception is not a litellm/openai exception (bug). Exit 0: it is mapped.
contrast_completion.py does the same with litellm.acompletion(stream=True) on bedrock/invoke/ (mapped).
