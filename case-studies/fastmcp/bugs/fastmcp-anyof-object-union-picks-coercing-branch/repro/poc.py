import fastmcp
from pydantic import TypeAdapter

from fastmcp.utilities.json_schema_type import json_schema_to_type

# Only the first branch accepts {"a": 0}: the second requires "a" to be a boolean.
schema = {
    "anyOf": [
        {"type": "object", "properties": {"x": {}}},
        {"type": "object", "properties": {"a": {"type": "boolean"}}},
    ]
}
adapter = TypeAdapter(json_schema_to_type(schema))
result = adapter.validate_python({"a": 0})
print(fastmcp.__file__)
print(adapter.dump_python(result, mode="json"))
