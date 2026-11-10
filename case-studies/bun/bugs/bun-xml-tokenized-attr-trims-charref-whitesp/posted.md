# Posted: https://github.com/oven-sh/bun/issues/48401

Filed by alpaylan through the GitHub API, 2026-11-09, at the owner's request. Text as posted:

---

## `Bun.XML.parse` drops a `&#xA;` at the end of an `NMTOKEN` attribute value

### What version of Bun is running?

1.4.4+38afee6c8 (also main 2596ac5d3)

### What platform is your computer?

Linux 7.0.0-1014-aws x86_64 x86_64

### What steps can reproduce the bug?

`xml-attr.js`:

```js
// v is declared NMTOKEN; &#xA; is a character reference to a newline.
const xml = '<!DOCTYPE r [<!ATTLIST r v NMTOKEN #IMPLIED>]><r v="a&#xA;"/>';
console.log(JSON.stringify(Bun.XML.parse(xml)));
```

```sh
bun xml-attr.js
```

### What is the expected behavior?

```
{"r":{"@v":"a\n"}}
```

XML 1.0 §3.3.3 (https://www.w3.org/TR/xml/#AVNormalize): "For a character reference, append the referenced character to the normalized value." For a type other than CDATA, the processor then discards "leading and trailing space (#x20) characters". A newline is not #x20, so it stays. Python's expat gives `'a\n'`. `docs/runtime/xml.mdx` says the parser "normalizes attribute values and applies attribute defaults declared in the internal subset."

### What do you see instead?

```
{"r":{"@v":"a"}}
```

### Additional information

`import d from "./x.xml"` gives the same `"a"`. `NMTOKENS` and enumerated types such as `(a|b)` lose these characters too, and so do `&#x9;` and `&#xD;` at either edge. With the attribute declared `CDATA`, or with no DTD, the result is `"a\n"`. A `&#32;` at the edge is trimmed, and the spec requires that.

