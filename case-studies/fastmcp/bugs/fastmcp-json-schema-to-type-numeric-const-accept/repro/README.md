# Repro: json_schema_to_type accepts true for a numeric const/enum and turns it into 1

Run with any Python that imports the fastmcp build under test:

    python mre.py

Bug present:
    {'const': 1} -> accepted, returned 1
    {'enum': [1]} -> accepted, returned 1
    {'anyOf': [{'const': 1}, {}]} -> accepted, returned 1
    structured_content: {'flag': True}
    data: Root(flag=1)

Bug absent (matches jsonschema Draft202012Validator):
    {'const': 1} -> rejected: ValidationError
    {'enum': [1]} -> rejected: ValidationError
    {'anyOf': [{'const': 1}, {}]} -> accepted, returned True
    structured_content: {'flag': True}
    data: Root(flag=True)

Checked 2026-10-28: reproduces on main 92c4c8d7, v4.1.4, 3.4.11 (pydantic 2.14.1). Not on the patched build.
