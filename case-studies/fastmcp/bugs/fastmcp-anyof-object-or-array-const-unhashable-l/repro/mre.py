from fastmcp.utilities.json_schema_type import json_schema_to_type

json_schema_to_type({"anyOf": [{"const": {}}, {"const": 1}]})
