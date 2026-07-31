//! JSON Pointer / JSON Patch / JSON Merge Patch (RFC 6901 / RFC 6902 / RFC 7396).
//! Source: cJSON_Utils.c / cJSON_Utils.h.
//!
//! Phase 6a (this session): JSON Pointer only - `get_pointer`,
//! `get_pointer_case_sensitive`, `find_pointer_from_object_to`. JSON Patch
//! and JSON Merge Patch are separate Phase 6 sessions (largest chunk of the
//! codebase - split by feature per the roadmap, not ported in one sitting).
//!
//! Design notes (see DECISIONS.md):
//! - `cJSONUtils_FindPointerFromObjectTo` relies on C pointer identity
//!   (`object == target`) to recognize the target node during the tree walk.
//!   Rust has a direct equivalent for shared references: `std::ptr::eq`.
//!   This only makes sense (as in C) when `target` is actually a reference
//!   *into* `object`'s own tree - passing an equal-but-separately-owned
//!   `Value` will correctly find nothing, matching upstream's pointer
//!   semantics rather than a value-equality search.
//! - No `unsafe`; pointer segments are decoded during comparison (mirroring
//!   `compare_pointers`) rather than via in-place byte mutation
//!   (`decode_pointer_inplace`, which Phase 6b's Patch code will need).

use crate::value::Value;

/// Resolves an RFC 6901 JSON Pointer against `value`, case-insensitively on
/// object keys. Mirrors `cJSONUtils_GetPointer`.
pub fn get_pointer<'a>(value: &'a Value, pointer: &str) -> Option<&'a Value> {
    get_item_from_pointer(value, pointer, false)
}

/// Case-sensitive variant. Mirrors `cJSONUtils_GetPointerCaseSensitive`.
pub fn get_pointer_case_sensitive<'a>(value: &'a Value, pointer: &str) -> Option<&'a Value> {
    get_item_from_pointer(value, pointer, true)
}

/// Mirrors get_item_from_pointer (cJSON_Utils.c:301-346): walks `/`-separated
/// path tokens, indexing into arrays by decimal index and objects by
/// (possibly escape-decoded) key.
fn get_item_from_pointer<'a>(object: &'a Value, pointer: &str, case_sensitive: bool) -> Option<&'a Value> {
    let mut current = object;
    let mut rest = pointer;

    while let Some(after_slash) = rest.strip_prefix('/') {
        let seg_end = after_slash.find('/').unwrap_or(after_slash.len());
        let segment = &after_slash[..seg_end];

        current = match current {
            Value::Array(items) => {
                let index = decode_array_index_from_pointer(segment)?;
                items.get(index)?
            }
            Value::Object(pairs) => {
                pairs
                    .iter()
                    .find(|(k, _)| compare_pointer_segment(k, segment, case_sensitive))
                    .map(|(_, v)| v)?
            }
            _ => return None,
        };

        rest = &after_slash[seg_end..];
    }

    Some(current)
}

/// Mirrors decode_array_index_from_pointer (cJSON_Utils.c:274-299): plain
/// decimal digits only, no leading zeros (except the single digit "0"
/// itself), and no support for RFC 6901's "-" (append) token - upstream
/// doesn't special-case it, so neither do we (it simply fails to parse as a
/// digit and the lookup returns `None`, matching upstream returning NULL).
fn decode_array_index_from_pointer(segment: &str) -> Option<usize> {
    let bytes = segment.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    if bytes[0] == b'0' && bytes.len() > 1 {
        return None;
    }

    let mut index: usize = 0;
    let mut pos = 0;
    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
        index = index.checked_mul(10)?.checked_add((bytes[pos] - b'0') as usize)?;
        pos += 1;
    }
    if pos != bytes.len() {
        return None;
    }
    Some(index)
}

/// Mirrors compare_pointers (cJSON_Utils.c:120-155): compares `name`
/// (an already-decoded object key) against `segment` (the raw, still
/// `~0`/`~1`-escaped pointer token, pre-truncated by the caller to end at
/// the next unescaped `/`), decoding escapes on the fly rather than
/// allocating a decoded copy first.
fn compare_pointer_segment(name: &str, segment: &str, case_sensitive: bool) -> bool {
    let name: Vec<char> = name.chars().collect();
    let seg: Vec<char> = segment.chars().collect();
    let mut ni = 0;
    let mut pi = 0;

    while ni < name.len() && pi < seg.len() {
        if seg[pi] == '~' {
            let escaped = seg.get(pi + 1).copied();
            let ok = (escaped == Some('0') && name[ni] == '~') || (escaped == Some('1') && name[ni] == '/');
            if !ok {
                return false;
            }
            pi += 1; // consume the extra escape-code character ('0'/'1'); the
                      // loop's own `pi += 1` below consumes the '~' itself.
        } else {
            let matches = if case_sensitive {
                name[ni] == seg[pi]
            } else {
                name[ni].to_ascii_lowercase() == seg[pi].to_ascii_lowercase()
            };
            if !matches {
                return false;
            }
        }
        ni += 1;
        pi += 1;
    }

    // Both must be fully consumed - one ending before the other means the
    // key and the pointer token had different (decoded) lengths.
    ni == name.len() && pi == seg.len()
}

/// Constructs the RFC 6901 pointer from `object` (the tree root) down to
/// `target`, identified by reference identity (`std::ptr::eq`) - `target`
/// must be a reference into `object`'s own tree, exactly as in upstream's
/// pointer-identity walk. Mirrors `cJSONUtils_FindPointerFromObjectTo`
/// (cJSON_Utils.c:198-260). Returns `None` if `target` isn't reachable from
/// `object` (upstream: NULL).
pub fn find_pointer_from_object_to(object: &Value, target: &Value) -> Option<String> {
    if std::ptr::eq(object, target) {
        return Some(String::new());
    }

    match object {
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                if let Some(suffix) = find_pointer_from_object_to(child, target) {
                    return Some(format!("/{}{}", index, suffix));
                }
            }
            None
        }
        Value::Object(pairs) => {
            for (key, child) in pairs.iter() {
                if let Some(suffix) = find_pointer_from_object_to(child, target) {
                    return Some(format!("/{}{}", encode_pointer_segment(key), suffix));
                }
            }
            None
        }
        _ => None,
    }
}

/// Mirrors encode_string_as_pointer (cJSON_Utils.c:173-196): escapes `~` as
/// `~0` and `/` as `~1`.
fn encode_pointer_segment(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for ch in key.chars() {
        match ch {
            '~' => out.push_str("~0"),
            '/' => out.push_str("~1"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    // RFC 6901 §5 example document, also used verbatim by upstream's
    // tests/old_utils_tests.c (pointer_tests).
    fn rfc6901_example() -> Value {
        parse(
            r#"{
                "foo": ["bar", "baz"],
                "": 0,
                "a/b": 1,
                "c%d": 2,
                "e^f": 3,
                "g|h": 4,
                "i\\j": 5,
                "k\"l": 6,
                " ": 7,
                "m~n": 8
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn empty_pointer_returns_whole_document() {
        let root = rfc6901_example();
        assert_eq!(get_pointer(&root, ""), Some(&root));
    }

    #[test]
    fn resolves_object_and_array_paths() {
        let root = rfc6901_example();
        assert_eq!(get_pointer(&root, "/foo"), root.object_get("foo"));
        assert_eq!(get_pointer(&root, "/foo/0").unwrap().as_str(), Some("bar"));
        assert_eq!(get_pointer(&root, "/foo/1").unwrap().as_str(), Some("baz"));
    }

    #[test]
    fn resolves_tricky_key_names_verbatim() {
        let root = rfc6901_example();
        assert_eq!(get_pointer(&root, "/"), root.object_get(""));
        assert_eq!(get_pointer(&root, "/c%d"), root.object_get("c%d"));
        assert_eq!(get_pointer(&root, "/e^f"), root.object_get("e^f"));
        assert_eq!(get_pointer(&root, "/g|h"), root.object_get("g|h"));
        assert_eq!(get_pointer(&root, "/i\\j"), root.object_get("i\\j"));
        assert_eq!(get_pointer(&root, "/k\"l"), root.object_get("k\"l"));
        assert_eq!(get_pointer(&root, "/ "), root.object_get(" "));
    }

    #[test]
    fn resolves_escaped_tilde_and_slash() {
        let root = rfc6901_example();
        assert_eq!(get_pointer(&root, "/a~1b"), root.object_get("a/b"));
        assert_eq!(get_pointer(&root, "/m~0n"), root.object_get("m~n"));
    }

    #[test]
    fn missing_path_returns_none() {
        let root = rfc6901_example();
        assert_eq!(get_pointer(&root, "/nope"), None);
        assert_eq!(get_pointer(&root, "/foo/99"), None);
    }

    #[test]
    fn array_index_rejects_leading_zero() {
        let arr = Value::Array(vec![Value::number(1.0), Value::number(2.0)]);
        assert_eq!(get_pointer(&arr, "/0"), Some(&Value::number(1.0)));
        assert_eq!(get_pointer(&arr, "/00"), None);
        assert_eq!(get_pointer(&arr, "/01"), None);
    }

    #[test]
    fn case_sensitivity_default_is_insensitive_matches_upstream_default() {
        let obj = Value::Object(vec![("Name".to_string(), Value::string("cJSON"))]);
        assert_eq!(get_pointer(&obj, "/name").unwrap().as_str(), Some("cJSON"));
        assert_eq!(get_pointer_case_sensitive(&obj, "/name"), None);
        assert_eq!(
            get_pointer_case_sensitive(&obj, "/Name").unwrap().as_str(),
            Some("cJSON")
        );
    }

    // --- find_pointer_from_object_to: vectors from upstream's misc_tests ---

    #[test]
    fn finds_pointer_to_self() {
        let root = rfc6901_example();
        assert_eq!(find_pointer_from_object_to(&root, &root), Some(String::new()));
    }

    #[test]
    fn finds_pointer_to_array_element() {
        let root = Value::Object(vec![(
            "numbers".to_string(),
            Value::Array((0..10).map(|n| Value::number(n as f64)).collect()),
        )]);
        let numbers = root.object_get("numbers").unwrap();
        let six = numbers.array_get(6).unwrap();
        assert_eq!(find_pointer_from_object_to(&root, numbers), Some("/numbers".to_string()));
        assert_eq!(find_pointer_from_object_to(&root, six), Some("/numbers/6".to_string()));
    }

    #[test]
    fn finds_pointer_and_escapes_tilde_and_slash_in_keys() {
        let obj1 = Value::Object(vec![("m~n".to_string(), Value::string("m~n"))]);
        let target1 = obj1.object_get("m~n").unwrap();
        assert_eq!(find_pointer_from_object_to(&obj1, target1), Some("/m~0n".to_string()));

        let obj2 = Value::Object(vec![("m/n".to_string(), Value::string("m/n"))]);
        let target2 = obj2.object_get("m/n").unwrap();
        assert_eq!(find_pointer_from_object_to(&obj2, target2), Some("/m~1n".to_string()));
    }

    #[test]
    fn unreachable_target_returns_none() {
        let root = Value::Object(vec![("a".to_string(), Value::number(1.0))]);
        let unrelated = Value::number(1.0); // equal by value, but not in root's tree
        assert_eq!(find_pointer_from_object_to(&root, &unrelated), None);
    }
}
