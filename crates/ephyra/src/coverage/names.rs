// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The profile names section: mangled function names keyed by the MD5-based
//! `NameRef` the data records carry.

use std::collections::HashMap;

use super::mapping::uleb;

/// Parse the `__llvm_prf_names` blob: a sequence of chunks, each
/// `uleb128 uncompressed_size, uleb128 compressed_size, bytes`, where a
/// compressed size of zero means the bytes are plain and otherwise they are
/// zlib-compressed. Names within a chunk are separated by `\x01`.
pub(crate) fn parse(blob: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut pos = 0;
    while pos < blob.len() {
        let Ok((uncompressed, next)) = uleb(blob, pos) else { break };
        let Ok((compressed, next)) = uleb(blob, next) else { break };
        let (chunk, after) = if compressed == 0 {
            let end = (next + uncompressed as usize).min(blob.len());
            (blob[next..end].to_vec(), end)
        } else {
            let end = (next + compressed as usize).min(blob.len());
            let inflated =
                miniz_oxide::inflate::decompress_to_vec_zlib(&blob[next..end]).unwrap_or_default();
            (inflated, end)
        };
        for name in chunk.split(|b| *b == 1).filter(|n| !n.is_empty()) {
            names.push(String::from_utf8_lossy(name).into_owned());
        }
        pos = after;
    }
    names
}

/// Map `NameRef` (the low 64 bits of the MD5 of the name) to the name.
pub(crate) fn by_ref(names: &[String]) -> HashMap<u64, &str> {
    names.iter().map(|n| (name_ref(n.as_bytes()), n.as_str())).collect()
}

/// LLVM's `IndexedInstrProf::ComputeHash`: the first eight bytes of the MD5
/// digest as a little-endian integer. Keys function names and filename
/// tables alike.
pub(crate) fn name_ref(bytes: &[u8]) -> u64 {
    let digest = md5(bytes);
    u64::from_le_bytes(digest[..8].try_into().expect("8 bytes"))
}

/// The mangled symbol of a PGO function name. Local functions carry a
/// `<file>:` prefix before it.
fn symbol(name: &str) -> &str {
    match name.find(":_") {
        Some(i) if name[i + 1..].starts_with("_R") || name[i + 1..].starts_with("_ZN") => {
            &name[i + 1..]
        },
        _ => name,
    }
}

/// A demangled, prefix-stripped version of a PGO function name.
pub(crate) fn pretty(name: &str) -> String {
    format!("{:#}", rustc_demangle::demangle(symbol(name)))
}

/// What the function is an instantiation of, the same text for all of its
/// instantiations: the path of its definition, starting with the name of the
/// crate. It has no generic arguments and no types; a module numbers its
/// impls, so that tells them apart. Two builds of one crate in a binary, such
/// as the library with and without its tests, give the same text. `None`
/// unless the name has v0 mangling, which instrumented Rust code has.
pub(crate) fn definition(name: &str) -> Option<String> {
    let symbol = symbol(name).strip_prefix("_R").filter(|symbol| symbol.is_ascii())?;
    let mut path = String::new();
    V0 { symbol, at: 0 }.path(Some(&mut path))?;
    Some(path)
}

/// A reader of the v0 mangling (RFC 2603). Each method reads past one
/// production of its grammar.
struct V0<'a> {
    symbol: &'a str,
    at: usize,
}

impl<'a> V0<'a> {
    fn next(&mut self) -> Option<u8> {
        let byte = *self.symbol.as_bytes().get(self.at)?;
        self.at += 1;
        Some(byte)
    }

    fn eat(&mut self, tag: u8) -> bool {
        let eaten = self.symbol.as_bytes().get(self.at) == Some(&tag);
        self.at += usize::from(eaten);
        eaten
    }

    /// The text up to a `_`.
    fn number(&mut self) -> Option<&'a str> {
        let rest = self.symbol.get(self.at..)?;
        let end = rest.find('_')?;
        self.at += end + 1;
        Some(&rest[..end])
    }

    /// `tag` and the number after it, or nothing.
    fn tagged(&mut self, tag: u8) -> Option<&'a str> {
        let start = self.at;
        if self.eat(tag) {
            self.number()?;
        }
        Some(&self.symbol[start..self.at])
    }

    /// Items up to an `E`.
    fn list(&mut self, mut item: impl FnMut(&mut Self) -> Option<()>) -> Option<()> {
        while !self.eat(b'E') {
            item(self)?;
        }
        Some(())
    }

    /// The identifier and its disambiguator, as `name#disambiguator`.
    fn identifier(&mut self, out: Option<&mut String>) -> Option<()> {
        let disambiguator = self.tagged(b's')?;
        self.eat(b'u');
        let start = self.at;
        if !self.eat(b'0') {
            while self.symbol.as_bytes().get(self.at).is_some_and(u8::is_ascii_digit) {
                self.at += 1;
            }
        }
        let len: usize = self.symbol[start..self.at].parse().ok()?;
        if len > 0 {
            self.eat(b'_');
        }
        let name = self.symbol.get(self.at..self.at.checked_add(len)?)?;
        self.at += len;
        if let Some(out) = out {
            out.extend([name, "#", disambiguator]);
        }
        Some(())
    }

    /// The definition that the path leads to goes to `out`, if there is one.
    fn path(&mut self, mut out: Option<&mut String>) -> Option<()> {
        let start = self.at;
        match self.next()? {
            b'C' => {
                let mut root = String::new();
                self.identifier(Some(&mut root))?;
                if let Some(out) = out {
                    // The disambiguator tells builds of the crate apart.
                    out.push_str(root.split('#').next()?);
                }
                Some(())
            },
            b'N' => {
                let namespace = self.next()?;
                self.path(out.as_deref_mut())?;
                if let Some(out) = out.as_deref_mut() {
                    out.push_str("::");
                    out.push(namespace.into());
                }
                self.identifier(out)
            },
            b'I' => {
                self.path(out)?;
                self.list(Self::generic_argument)
            },
            b'M' => {
                self.impl_path(out)?;
                self.ty()
            },
            b'X' => {
                self.impl_path(out)?;
                self.ty()?;
                self.path(None)
            },
            // A method that the trait provides.
            b'Y' => {
                self.ty()?;
                self.path(out)
            },
            b'B' => {
                let target = self.number()?;
                match out {
                    // Only what comes before can be referred to.
                    Some(_) => V0 { symbol: &self.symbol[..start], at: base_62(target)? }.path(out),
                    None => Some(()),
                }
            },
            _ => None,
        }
    }

    /// The module of an impl and the number of the impl in it. The compiler
    /// writes an uninstantiated generic impl of a trait as a path of namespace
    /// `I` with the parameters as arguments, and any other impl like this.
    fn impl_path(&mut self, mut out: Option<&mut String>) -> Option<()> {
        if self.symbol.as_bytes().get(self.at) == Some(&b'I') {
            return self.path(out);
        }
        let number = self.tagged(b's')?;
        self.path(out.as_deref_mut())?;
        if let Some(out) = out {
            out.extend(["::I#", number]);
        }
        Some(())
    }

    fn generic_argument(&mut self) -> Option<()> {
        if self.eat(b'K') {
            self.constant()
        } else if self.tagged(b'L')?.is_empty() {
            self.ty()
        } else {
            Some(())
        }
    }

    fn ty(&mut self) -> Option<()> {
        let start = self.at;
        match self.next()? {
            b'a'..=b'z' => Some(()),
            b'A' => {
                self.ty()?;
                self.constant()
            },
            b'S' | b'P' | b'O' => self.ty(),
            b'R' | b'Q' => {
                self.tagged(b'L')?;
                self.ty()
            },
            b'T' => self.list(Self::ty),
            b'F' => {
                self.tagged(b'G')?;
                self.eat(b'U');
                if self.eat(b'K') && !self.eat(b'C') {
                    self.identifier(None)?;
                }
                self.list(Self::ty)?;
                self.ty()
            },
            b'D' => {
                self.tagged(b'G')?;
                self.list(|this| {
                    this.path(None)?;
                    while this.eat(b'p') {
                        this.identifier(None)?;
                        this.ty()?;
                    }
                    Some(())
                })?;
                self.tagged(b'L')?;
                Some(())
            },
            _ => {
                self.at = start;
                self.path(None)
            },
        }
    }

    fn constant(&mut self) -> Option<()> {
        match self.next()? {
            b'p' => Some(()),
            b'R' | b'Q' => self.constant(),
            b'A' | b'T' => self.list(Self::constant),
            b'V' => {
                self.path(None)?;
                match self.next()? {
                    b'U' => Some(()),
                    b'T' => self.list(Self::constant),
                    b'S' => {
                        self.list(|this| {
                            this.identifier(None)?;
                            this.constant()
                        })
                    },
                    _ => None,
                }
            },
            // A type and its value, or a back reference.
            _ => self.number().map(|_| ()),
        }
    }
}

/// The value of a base 62 number, which is without digits for zero.
fn base_62(digits: &str) -> Option<usize> {
    if digits.is_empty() {
        return Some(0);
    }
    let value = digits.bytes().try_fold(0usize, |value, digit| {
        let digit = match digit {
            b'0'..=b'9' => digit - b'0',
            b'a'..=b'z' => digit - b'a' + 10,
            b'A'..=b'Z' => digit - b'A' + 36,
            _ => return None,
        };
        value.checked_mul(62)?.checked_add(digit.into())
    })?;
    value.checked_add(1)
}

/// RFC 1321 MD5, needed only to recover names from their hashes.
pub(crate) fn md5(input: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> =
        (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32).collect();
    let (mut a0, mut b0, mut c0, mut d0) =
        (0x6745_2301u32, 0xefcd_ab89u32, 0x98ba_dcfeu32, 0x1032_5476u32);
    let mut message = input.to_vec();
    let bit_len = (input.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_le_bytes());
    for chunk in message.chunks(64) {
        let m: Vec<u32> =
            chunk.chunks(4).map(|w| u32::from_le_bytes(w.try_into().expect("4 bytes"))).collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    out[..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..].copy_from_slice(&d0.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(d: [u8; 16]) -> String {
        d.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn instantiations_share_a_definition() {
        let definitions: [(&str, &[&str]); 6] = [
            (
                // A closure of `a::f`, for a trait object, a function pointer and `_`.
                "a::vf#::C#",
                &[
                    "_RNCINvCs4ENdUi5qgPG_1a1fDG_INtNtNtCsjIpoRzUjyHQ_4core3ops8function2FnTRL0_hEEp6OutputRL0_hEL_E0B4_",
                    "_RNCINvCs4ENdUi5qgPG_1a1fFG_UK8C_unwindRL0_ePhOSxEThQbEE0B4_",
                    "_RNCINvCs4ENdUi5qgPG_1a1fpE0B4_",
                ],
            ),
            (
                // `a::k::<'<', -3, 4>` and `a::k::<_, _, _>`, of two builds of `a`.
                "a::vk#",
                &["_RINvCs4ENdUi5qgPG_1a1kKc3c_Kan3_Kj4_EB2_", "_RINvCs9ENdUi5qgPG_1a1kKpKpKpEB2_"],
            ),
            ("a::tgre_6ka8i#::t_3d#::vf#", &["_RINvNtNtCs4ENdUi5qgPG_1au9gre_6ka8i3__3d1fhEB4_"]),
            (
                // A method that the trait provides, for `t::Sq` and for `_`.
                "geo::tShape#::vdescribe#",
                &[
                    "_RNvYNtCsioAtl7YS8i6_1t2SqNtCsg0mqAflEUTN_3geo5Shape8describeB4_",
                    "_RNvYpNtCsg0mqAflEUTN_3geo5Shape8describeB5_",
                ],
            ),
            // Two impls of one trait by one macro: `impl<T> PartialEq<Wr<u16>>
            // for Both<T, u16>`, for `i64` and for `_`, and the same for `u32`.
            (
                "conv::I#sk_::veq#",
                &[
                    "_RNvXsk_CsahIAEWYdudw_4convINtB5_4BothxtEINtNtCsjIpoRzUjyHQ_4core3cmp9PartialEqINtB5_2WrtEE2eqCs2ZcE9zVa1Nt_1t",
                    "_RNvXINICsahIAEWYdudw_4convsk_0pEINtB5_4BothptEINtNtCsjIpoRzUjyHQ_4core3cmp9PartialEqINtB5_2WrtEE2eqB5_",
                ],
            ),
            (
                "conv::I#sl_::veq#",
                &[
                    "_RNvXINICsahIAEWYdudw_4convsl_0pEINtB5_4BothpmEINtNtCsjIpoRzUjyHQ_4core3cmp9PartialEqINtB5_2WrmEE2eqB5_",
                ],
            ),
        ];
        for (path, names) in definitions {
            for name in names {
                assert_eq!(definition(name).as_deref(), Some(path), "{name}");
            }
        }
        for name in ["main", "_ZN3foo3barE", "_RNvB_3foo", "_RNvC1a", "_RN"] {
            assert_eq!(definition(name), None);
        }
    }

    #[test]
    fn md5_matches_reference_vectors() {
        assert_eq!(hex(md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(md5(b"The quick brown fox jumps over the lazy dog")),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        let long = "a".repeat(200);
        assert_eq!(hex(md5(long.as_bytes())), "887f30b43b2867f4a9accceee7d16e6c");
    }

    #[test]
    fn names_blob_round_trip_uncompressed() {
        let payload = b"main\x01foo::bar";
        let mut blob = vec![payload.len() as u8, 0];
        blob.extend_from_slice(payload);
        let names = parse(&blob);
        assert_eq!(names, vec!["main", "foo::bar"]);
        let map = by_ref(&names);
        assert_eq!(map[&name_ref(b"main")], "main");
    }

    #[test]
    fn names_blob_round_trip_compressed() {
        let payload = b"_ZN3foo3barE\x01baz";
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(payload, 6);
        let mut blob = vec![payload.len() as u8, compressed.len() as u8];
        blob.extend_from_slice(&compressed);
        assert_eq!(parse(&blob), vec!["_ZN3foo3barE", "baz"]);
        assert_eq!(pretty("src/lib.rs:_ZN3foo3barE"), "foo::bar");
    }
}
