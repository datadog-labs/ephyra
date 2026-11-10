from fastmcp.utilities.json_schema import dereference_refs
print(dereference_refs({"$ref": "#/$defs/Any", "$defs": {"Any": True}}))
