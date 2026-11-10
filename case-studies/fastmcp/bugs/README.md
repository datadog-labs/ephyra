# Bugs: fastmcp

The bugs the case studies found in fastmcp. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [json_schema_to_type: const/enum accept true for 1, and an object-valued const in a union raises TypeError](fastmcp-anyof-object-or-array-const-unhashable-l/) | fixed | [link](https://github.com/PrefectHQ/fastmcp/issues/6121) |
| [result.data is built from an anyOf branch that coerces the value (0 becomes False)](fastmcp-anyof-object-union-picks-coercing-branch/) | open | [link](https://github.com/PrefectHQ/fastmcp/issues/6136) |
| [OAuth client redirect carries two `iss` parameters when the registered redirect_uri already repeats `iss`](fastmcp-build-client-redirect-keeps-duplicate-is/) | fixed | [link](https://github.com/PrefectHQ/fastmcp/issues/6297) |
| [OAuth client redirect turns a registered `myapp://` redirect_uri into `myapp:?...`](fastmcp-client-redirect-drops-empty-authority/) | fixed | [link](https://github.com/PrefectHQ/fastmcp/issues/6296) |
| [json_schema_to_type rejects a valid object when a property's default doesn't match its own type](fastmcp-dataclass-path-validates-invalid-propert/) | open | [link](https://github.com/PrefectHQ/fastmcp/issues/6123) |
| [tools/list fails for the whole server when a tool's root $ref points to a boolean schema](fastmcp-dereference-root-ref-to-boolean-schema-c/) | open | [link](https://github.com/PrefectHQ/fastmcp/issues/6125) |
| [json_schema_to_type applies items to the prefixItems positions](fastmcp-json-schema-to-type-items-applied-to-pre/) | open | [link](https://github.com/PrefectHQ/fastmcp/issues/6122) |
| [json_schema_to_type: const/enum accept true for 1, and an object-valued const in a union raises TypeError](fastmcp-json-schema-to-type-numeric-const-accept/) | fixed | [link](https://github.com/PrefectHQ/fastmcp/issues/6121) |
| [`Client(mode="legacy", cache=CacheConfig(default_ttl_ms=...))` serves cached listings, although the cache is documented as inert on legacy connections](fastmcp-legacy-connection-response-cache-not-ine/) | fixed | [link](https://github.com/PrefectHQ/fastmcp/issues/6298) |
| [`logging/setLevel` is ignored for `ctx.log()` calls from tools of a mounted server](fastmcp-mounted-server-ignores-client-setlevel/) | open | [link](https://github.com/PrefectHQ/fastmcp/issues/6299) |
| [`ctx.elicit()` multi-select with `[[...]]` sends `items` without `"type": "string"`, which the 2025-11-25 schema requires](fastmcp-multi-select-elicit-items-missing-string/) | fixed | [link](https://github.com/PrefectHQ/fastmcp/issues/6300) |
| [from_openapi drops a query value when an object query parameter has a property named like another parameter](fastmcp-openapi-exploded-object-query-property-o/) | fixed | [link](https://github.com/PrefectHQ/fastmcp/issues/6149) |
| [from_openapi lists resource templates for non-identifier path parameters (e.g. {$top}) that can never be read](fastmcp-openapi-template-non-identifier-path-par/) | open | [link](https://github.com/PrefectHQ/fastmcp/issues/6150) |
| [A proxy served over stdio forwards only some of the backend's `ctx.log()` messages](fastmcp-proxy-drops-forwarded-notifications-on-b/) | open | [link](https://github.com/PrefectHQ/fastmcp/issues/6303) |
| [Mutually recursive Pydantic return types are hydrated as the wrong class in result.data](fastmcp-recursive-definition-forward-ref-unresol/) | open | [link](https://github.com/PrefectHQ/fastmcp/issues/6135) |
| [Redirect URI helpers accept raw-string URIs that resolve to a different host or path (backslash, trailing space/C0, IPv4 shorthand, percent-encoded or IDNA hosts)](fastmcp-redirect-wildcard-host-backslash-bypass/) | open | [link](https://github.com/PrefectHQ/fastmcp/issues/6295) |
| [A supporting file named `SKILL.md` in a subfolder is listed as a separate skill and makes `sync_skills()` fail](fastmcp-sync-skills-treats-nested-skill-md-suppo/) | open | [link](https://github.com/PrefectHQ/fastmcp/issues/6302) |
| [json_schema_to_type drops additionalProperties keys when the schema has properties but no type](fastmcp-untyped-properties-ignores-additional-pr/) | open | [link](https://github.com/PrefectHQ/fastmcp/issues/6124) |
| [Versionless lookup through a mount does not fall back to the enabled version when the child server has list-filtering middleware](fastmcp-version-fallback-runs-mounted-server-mid/) | open | [link](https://github.com/PrefectHQ/fastmcp/issues/6301) |
