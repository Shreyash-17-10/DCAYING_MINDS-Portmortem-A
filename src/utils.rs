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
                name[ni].eq_ignore_ascii_case(&seg[pi])
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
#[allow(clippy::approx_constant)]
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

// ============================================================================
// Phase 6b: JSON Patch (RFC 6902) and JSON Merge Patch (RFC 7396) - apply
// side only. Diff/patch-*generation* (`cJSONUtils_GeneratePatches`,
// `cJSONUtils_GenerateMergePatch`) is not ported - see DECISIONS.md for the
// scope note. This covers `apply_patch`, `cJSONUtils_ApplyPatches[CaseSensitive]`,
// and `merge_patch`/`cJSONUtils_MergePatch[CaseSensitive]` from cJSON_Utils.c.
// ============================================================================

use crate::value::compare;

/// Errors from applying a single JSON Patch operation or a JSON Patch
/// document. Named for what went wrong rather than mirroring C's numeric
/// `status` codes (`apply_patch` returns 0/2/3/4/5/7/9/10/11/13) directly,
/// though each variant's doc comment cites the status it replaces for
/// traceability back to cJSON_Utils.c.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    /// Patch entry is missing a string `"path"`. Mirrors status 2.
    MalformedPatch,
    /// `"op"` is missing or isn't one of add/remove/replace/move/copy/test.
    /// Mirrors status 3.
    InvalidOperation,
    /// `move`/`copy` is missing a string `"from"`. Mirrors status 4.
    MissingFrom,
    /// `"from"` didn't resolve to anything in the document. Mirrors status 5.
    FromNotFound,
    /// `add`/`replace` is missing `"value"`. Mirrors status 7.
    MissingValue,
    /// `"path"` (or its parent) didn't resolve to anything in the document.
    /// Mirrors statuses 9 and 13 (parent lookup failure and
    /// remove/replace's old-item lookup failure respectively - both are
    /// "couldn't find the thing this op needs" and are collapsed into one
    /// variant here).
    PathNotFound,
    /// Array index in `"path"` (for `add`) is malformed or past the end of
    /// the array (append is only valid at exactly `array.len()`, or via the
    /// literal `"-"` token). Mirrors statuses 10 and 11.
    ArrayIndexInvalid,
    /// `test` operation: the value at `"path"` didn't match `"value"`.
    /// This isn't a malformed-patch error in the same sense as the others -
    /// it's the patch *correctly reporting* that its precondition failed,
    /// mirroring `apply_patch`'s `status = !compare_json(...)` path, which
    /// upstream folds into the same generic non-zero return as every other
    /// failure. Kept as its own variant here since a caller reasonably
    /// wants to distinguish "this doc doesn't match what I expected" from
    /// "this patch document is malformed".
    TestFailed,
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PatchError::MalformedPatch => write!(f, "malformed patch: missing \"path\""),
            PatchError::InvalidOperation => write!(f, "invalid or missing \"op\""),
            PatchError::MissingFrom => write!(f, "move/copy operation missing \"from\""),
            PatchError::FromNotFound => write!(f, "\"from\" path not found in document"),
            PatchError::MissingValue => write!(f, "add/replace operation missing \"value\""),
            PatchError::PathNotFound => write!(f, "\"path\" not found in document"),
            PatchError::ArrayIndexInvalid => write!(f, "array index out of bounds or malformed"),
            PatchError::TestFailed => write!(f, "test operation failed: value mismatch"),
        }
    }
}

impl std::error::Error for PatchError {}

enum PatchOp {
    Add,
    Remove,
    Replace,
    Move,
    Copy,
    Test,
}

/// Mirrors `get_object_item` (cJSON_Utils.c:730-738): the local `op`/`path`/
/// `value`/`from` lookups inside patch-handling code all respect the same
/// `case_sensitive` flag as the pointer resolution itself.
fn get_object_item<'a>(object: &'a Value, name: &str, case_sensitive: bool) -> Option<&'a Value> {
    if case_sensitive {
        object.object_get(name)
    } else {
        object.object_get_ci(name)
    }
}

/// Mirrors `decode_patch_operation` (cJSON_Utils.c:742-781).
fn decode_patch_operation(patch: &Value, case_sensitive: bool) -> Option<PatchOp> {
    let op = get_object_item(patch, "op", case_sensitive)?.as_str()?;
    match op {
        "add" => Some(PatchOp::Add),
        "remove" => Some(PatchOp::Remove),
        "replace" => Some(PatchOp::Replace),
        "move" => Some(PatchOp::Move),
        "copy" => Some(PatchOp::Copy),
        "test" => Some(PatchOp::Test),
        _ => None,
    }
}

/// Decodes `~0`/`~1` escapes in a single (already-isolated) pointer segment.
/// Mirrors `decode_pointer_inplace` (cJSON_Utils.c:349-378), done as a
/// fresh-string single pass instead of C's in-place buffer rewrite.
fn decode_pointer_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    let mut chars = segment.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '~' {
            match chars.peek() {
                Some('0') => {
                    out.push('~');
                    chars.next();
                }
                Some('1') => {
                    out.push('/');
                    chars.next();
                }
                _ => out.push('~'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Splits a pointer at its last `/` into `(parent_pointer, decoded_child_segment)`.
/// Mirrors the parent/child pointer split in `apply_patch`
/// (cJSON_Utils.c:961-972): `strrchr` for the last `/`, then
/// `decode_pointer_inplace` on the tail. Returns `None` if `path` contains
/// no `/` at all (mirrors `child_pointer == NULL`, i.e. an unreachable
/// "parent" for a patch operation - the top-level `""` path is always
/// intercepted separately in `apply_patch_inner` before this is called).
fn split_last_pointer_segment(path: &str) -> Option<(&str, String)> {
    let slash_pos = path.rfind('/')?;
    let parent_pointer = &path[..slash_pos];
    let child_raw = &path[slash_pos + 1..];
    Some((parent_pointer, decode_pointer_segment(child_raw)))
}

/// Mutable counterpart to `get_item_from_pointer`, used to navigate to the
/// parent container a patch operation will mutate. No `unsafe`: reassigning
/// `current` inside the loop relies on ordinary non-lexical-lifetime
/// reborrowing, not raw pointers.
fn get_pointer_mut<'a>(value: &'a mut Value, pointer: &str, case_sensitive: bool) -> Option<&'a mut Value> {
    let mut current = value;
    let mut rest = pointer;

    while let Some(after_slash) = rest.strip_prefix('/') {
        let seg_end = after_slash.find('/').unwrap_or(after_slash.len());
        let segment = &after_slash[..seg_end];

        current = match current {
            Value::Array(items) => {
                let index = decode_array_index_from_pointer(segment)?;
                items.get_mut(index)?
            }
            Value::Object(pairs) => pairs
                .iter_mut()
                .find(|(k, _)| compare_pointer_segment(k, segment, case_sensitive))
                .map(|(_, v)| v)?,
            _ => return None,
        };

        rest = &after_slash[seg_end..];
    }

    Some(current)
}

/// Detaches and returns the item at `path`. Mirrors `detach_path`
/// (cJSON_Utils.c). Only called for non-root paths (root removal/replacement
/// is handled directly in `apply_patch_inner`).
fn detach_by_pointer(object: &mut Value, path: &str, case_sensitive: bool) -> Option<Value> {
    let (parent_pointer, child_segment) = split_last_pointer_segment(path)?;
    let parent = get_pointer_mut(object, parent_pointer, case_sensitive)?;
    match parent {
        Value::Array(_) => {
            let index = decode_array_index_from_pointer(&child_segment)?;
            parent.array_detach(index)
        }
        Value::Object(_) => parent.object_detach(&child_segment, case_sensitive),
        _ => None,
    }
}

/// Applies a single JSON Patch operation object to `object` in place.
/// Mirrors `apply_patch` (cJSON_Utils.c:807-1036) function-for-function.
fn apply_patch_inner(object: &mut Value, patch: &Value, case_sensitive: bool) -> Result<(), PatchError> {
    let path = get_object_item(patch, "path", case_sensitive)
        .and_then(Value::as_str)
        .ok_or(PatchError::MalformedPatch)?;

    let opcode = decode_patch_operation(patch, case_sensitive).ok_or(PatchError::InvalidOperation)?;

    if let PatchOp::Test = opcode {
        let actual = get_item_from_pointer(object, path, case_sensitive);
        let expected = get_object_item(patch, "value", case_sensitive);
        return match (actual, expected) {
            (Some(a), Some(b)) if compare(a, b, case_sensitive) => Ok(()),
            _ => Err(PatchError::TestFailed),
        };
    }

    // Special case for the root: no parent to split off, so add/replace/
    // remove at "" are handled directly (mirrors cJSON_Utils.c:838-885).
    // Move/copy targeting the root fall through to the generic path below
    // (via `split_last_pointer_segment("")` returning `None`), which
    // produces the same `PathNotFound` outcome C reaches by a different
    // route (no `/` to split on).
    if path.is_empty() {
        match opcode {
            PatchOp::Remove => {
                // C overwrites the root node with a `cJSON_Invalid`-typed
                // sentinel struct (cJSON_Utils.c:843), which has no
                // observable-behavior equivalent variant in `Value` (it's
                // an internal "this node is gone" marker, not a JSON type).
                // `Value::Null` is the closest stand-in; documented here as
                // a deliberate, minor divergence rather than left implicit.
                *object = Value::Null;
                return Ok(());
            }
            PatchOp::Replace | PatchOp::Add => {
                let value = get_object_item(patch, "value", case_sensitive)
                    .ok_or(PatchError::MissingValue)?;
                *object = value.duplicate(true);
                return Ok(());
            }
            _ => return Err(PatchError::PathNotFound),
        }
    }

    // remove/replace: detach the old item first (mirrors cJSON_Utils.c:887-903).
    if matches!(opcode, PatchOp::Remove | PatchOp::Replace) {
        detach_by_pointer(object, path, case_sensitive).ok_or(PatchError::PathNotFound)?;
        if let PatchOp::Remove = opcode {
            return Ok(());
        }
    }

    // move/copy use "from"; add/replace use "value" (cJSON_Utils.c:905-957).
    let value = match opcode {
        PatchOp::Move => {
            let from = get_object_item(patch, "from", case_sensitive)
                .and_then(Value::as_str)
                .ok_or(PatchError::MissingFrom)?;
            detach_by_pointer(object, from, case_sensitive).ok_or(PatchError::FromNotFound)?
        }
        PatchOp::Copy => {
            let from = get_object_item(patch, "from", case_sensitive)
                .and_then(Value::as_str)
                .ok_or(PatchError::MissingFrom)?;
            let source = get_item_from_pointer(object, from, case_sensitive)
                .ok_or(PatchError::FromNotFound)?;
            source.duplicate(true)
        }
        PatchOp::Add | PatchOp::Replace => {
            let v = get_object_item(patch, "value", case_sensitive).ok_or(PatchError::MissingValue)?;
            v.duplicate(true)
        }
        PatchOp::Remove | PatchOp::Test => unreachable!("handled above"),
    };

    // Add "value" at "path" (cJSON_Utils.c:959-1023).
    let (parent_pointer, child_segment) = split_last_pointer_segment(path).ok_or(PatchError::PathNotFound)?;
    let parent = get_pointer_mut(object, parent_pointer, case_sensitive).ok_or(PatchError::PathNotFound)?;

    match parent {
        Value::Array(items) => {
            if child_segment == "-" {
                items.push(value);
            } else {
                let index = decode_array_index_from_pointer(&child_segment)
                    .ok_or(PatchError::ArrayIndexInvalid)?;
                // Mirrors insert_item_in_array (cJSON_Utils.c:693-728):
                // index == len is a valid append; index > len is an error,
                // unlike Value::array_insert's silent clamp (that method
                // exists for cJSON_InsertItemInArray parity elsewhere,
                // which *does* clamp - JSON Patch's `add` must not).
                if index > items.len() {
                    return Err(PatchError::ArrayIndexInvalid);
                }
                items.insert(index, value);
            }
            Ok(())
        }
        Value::Object(_) => {
            // Mirrors cJSON_Utils.c:1007-1016: delete any existing entry
            // under this key first, then add - so "add" transparently
            // replaces an existing key, same as upstream.
            parent.object_delete(&child_segment, case_sensitive);
            parent
                .object_push(child_segment, value)
                .map_err(|_| PatchError::PathNotFound)
        }
        _ => Err(PatchError::PathNotFound),
    }
}

/// Applies a single JSON Patch operation, case-insensitive key matching.
/// Mirrors the case-insensitive path through `apply_patch` as called by
/// `cJSONUtils_ApplyPatches`.
pub fn apply_patch(object: &mut Value, patch: &Value) -> Result<(), PatchError> {
    apply_patch_inner(object, patch, false)
}

/// Case-sensitive variant. Mirrors the case-sensitive path through
/// `apply_patch` as called by `cJSONUtils_ApplyPatchesCaseSensitive`.
pub fn apply_patch_case_sensitive(object: &mut Value, patch: &Value) -> Result<(), PatchError> {
    apply_patch_inner(object, patch, true)
}

fn apply_patches_inner(object: &mut Value, patches: &Value, case_sensitive: bool) -> Result<(), PatchError> {
    let items = patches.as_array().ok_or(PatchError::MalformedPatch)?;
    for patch in items {
        apply_patch_inner(object, patch, case_sensitive)?;
    }
    Ok(())
}

/// Applies a JSON Patch document (an array of operations) in order, stopping
/// at the first failure. Mirrors `cJSONUtils_ApplyPatches` (cJSON_Utils.c:
/// 1038-1065): **not transactional** - operations already applied before a
/// failure stay applied, matching upstream's behavior exactly rather than
/// rolling back.
pub fn apply_patches(object: &mut Value, patches: &Value) -> Result<(), PatchError> {
    apply_patches_inner(object, patches, false)
}

/// Case-sensitive variant. Mirrors `cJSONUtils_ApplyPatchesCaseSensitive`.
pub fn apply_patches_case_sensitive(object: &mut Value, patches: &Value) -> Result<(), PatchError> {
    apply_patches_inner(object, patches, true)
}

// ============================================================================
// JSON Merge Patch (RFC 7396) - apply side. Mirrors `merge_patch`
// (cJSON_Utils.c:1321-1379).
// ============================================================================

fn merge_patch_inner(target: Value, patch: &Value, case_sensitive: bool) -> Value {
    let patch_pairs = match patch.as_object() {
        Some(pairs) => pairs,
        // Scalar, array, or (already-filtered-elsewhere) null patch: replace
        // wholesale. Mirrors cJSON_Utils.c:1325-1330.
        None => return patch.duplicate(true),
    };

    let mut target = if target.is_object() { target } else { Value::object() };

    for (key, patch_child) in patch_pairs.iter().cloned() {
        if patch_child.is_null() {
            // RFC 7396: null in the patch means "delete this key".
            target.object_delete(&key, case_sensitive);
        } else {
            let existing = target
                .object_detach(&key, case_sensitive)
                .unwrap_or(Value::Null);

            let merged = merge_patch_inner(existing, &patch_child, case_sensitive);
            let _ = target.object_push(key, merged);
        }
    }

    target
}

/// Applies an RFC 7396 JSON Merge Patch to `target`, consuming it and
/// returning the merged result. Mirrors `cJSONUtils_MergePatch`
/// (case-insensitive key matching).
pub fn merge_patch(target: Value, patch: &Value) -> Value {
    merge_patch_inner(target, patch, false)
}

/// Case-sensitive variant. Mirrors `cJSONUtils_MergePatchCaseSensitive`.
pub fn merge_patch_case_sensitive(target: Value, patch: &Value) -> Value {
    merge_patch_inner(target, patch, true)
}

// ============================================================================
// Phase 7: sort_object, JSON Patch generation (RFC 6902), and JSON Merge Patch
// generation (RFC 7396). Closes the last functional gap vs upstream's public
// API — see DECISIONS.md §1 (scope table) and §6b.
// ============================================================================

/// Sorts an object's keys alphabetically (case-insensitive). Mirrors
/// `cJSONUtils_SortObject` (cJSON_Utils.c:1311-1314). If `value` is not an
/// Object, this is a no-op (matching upstream's NULL-guard behavior).
///
/// Design note: C implements this as a merge sort over an intrusive doubly-
/// linked list (`sort_list`, cJSON_Utils.c:484-593). Rust uses `Vec::sort_by`
/// (a stable, adaptive merge sort on a contiguous slice) — identical
/// stability guarantees, much less code, and better cache locality.
pub fn sort_object(value: &mut Value) {
    sort_object_inner(value, false);
}

/// Case-sensitive variant. Mirrors `cJSONUtils_SortObjectCaseSensitive`.
pub fn sort_object_case_sensitive(value: &mut Value) {
    sort_object_inner(value, true);
}

fn sort_object_inner(value: &mut Value, case_sensitive: bool) {
    if let Value::Object(pairs) = value {
        pairs.sort_by(|(a, _), (b, _)| {
            if case_sensitive {
                a.cmp(b)
            } else {
                a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase())
            }
        });
    }
}

/// Builds a single JSON Patch operation object. Mirrors `compose_patch`
/// (cJSON_Utils.c:1096-1134). Instead of C's manual `sprintf` path
/// concatenation, this uses Rust's `format!` with `encode_pointer_segment`.
fn compose_patch(patches: &mut Value, operation: &str, path: &str, suffix: Option<&str>, val: Option<&Value>) {
    let mut patch = Value::object();
    patch.object_push("op", Value::string(operation)).ok();

    let full_path = match suffix {
        None => path.to_string(),
        Some(s) => format!("{}/{}", path, encode_pointer_segment(s)),
    };
    patch.object_push("path", Value::string(full_path)).ok();

    if let Some(v) = val {
        patch.object_push("value", v.duplicate(true)).ok();
    }

    patches.array_push(patch).ok();
}

/// Public utility for building patch array entries. Mirrors
/// `cJSONUtils_AddPatchToArray` (cJSON_Utils.c:1136-1139).
pub fn add_patch_to_array(array: &mut Value, operation: &str, path: &str, value: Option<&Value>) {
    compose_patch(array, operation, path, None, value);
}

/// Recursive diff: produces a sequence of JSON Patch (RFC 6902) operations
/// that transforms `from` into `to`. Mirrors `create_patches`
/// (cJSON_Utils.c:1141-1279).
///
/// Key design difference from C: C's `create_patches` calls `sort_object`
/// on its inputs, mutating them as a side-effect. This Rust version works
/// on clones of the sorted pairs instead, avoiding mutation of the inputs
/// — documented as an intentional divergence since the C behavior is a
/// surprising side-effect callers shouldn't depend on (and upstream's own
/// header warns "NOTE: This modifies objects in 'from' and 'to' by sorting
/// the elements by their key").
fn create_patches(patches: &mut Value, path: &str, from: &Value, to: &Value, case_sensitive: bool) {
    // Different types → wholesale replace (cJSON_Utils.c:1148-1152).
    if std::mem::discriminant(from) != std::mem::discriminant(to) {
        compose_patch(patches, "replace", path, None, Some(to));
        return;
    }

    match (from, to) {
        (Value::Number(a), Value::Number(b)) => {
            if !compare_double_patch(*a, *b) {
                compose_patch(patches, "replace", path, None, Some(to));
            }
        }
        (Value::String(a), Value::String(b)) => {
            if a != b {
                compose_patch(patches, "replace", path, None, Some(to));
            }
        }
        (Value::Bool(a), Value::Bool(b)) => {
            if a != b {
                compose_patch(patches, "replace", path, None, Some(to));
            }
        }
        (Value::Array(from_items), Value::Array(to_items)) => {
            // Elements present in both: recurse (cJSON_Utils.c:1177-1190).
            let common_len = from_items.len().min(to_items.len());
            for i in 0..common_len {
                let new_path = format!("{}/{}", path, i);
                create_patches(patches, &new_path, &from_items[i], &to_items[i], case_sensitive);
            }

            // Extra elements in `from` → remove. C removes at the same
            // index repeatedly (because removal shifts elements), mirrored
            // here by always removing at `common_len` (cJSON_Utils.c:1192-1205).
            for _ in common_len..from_items.len() {
                let idx_str = common_len.to_string();
                compose_patch(patches, "remove", path, Some(&idx_str), None);
            }

            // Extra elements in `to` → add with "-" (append) token
            // (cJSON_Utils.c:1207-1210).
            for to_child in to_items.iter().skip(common_len) {
                compose_patch(patches, "add", path, Some("-"), Some(to_child));
            }
        }
        (Value::Object(_), Value::Object(_)) => {
            // Sort both sides by key for the merge-walk (mirrors
            // cJSON_Utils.c:1219-1220 calling sort_object on both).
            // We clone + sort to avoid mutating the inputs.
            let mut from_sorted = from.duplicate(true);
            let mut to_sorted = to.duplicate(true);
            sort_object_inner(&mut from_sorted, case_sensitive);
            sort_object_inner(&mut to_sorted, case_sensitive);

            let from_pairs = from_sorted.as_object().unwrap();
            let to_pairs = to_sorted.as_object().unwrap();

            let mut fi = 0;
            let mut ti = 0;

            while fi < from_pairs.len() || ti < to_pairs.len() {
                let diff = if fi >= from_pairs.len() {
                    std::cmp::Ordering::Greater // from exhausted, to has extra
                } else if ti >= to_pairs.len() {
                    std::cmp::Ordering::Less // to exhausted, from has extra
                } else {
                    let (ref fk, _) = from_pairs[fi];
                    let (ref tk, _) = to_pairs[ti];
                    if case_sensitive {
                        fk.cmp(tk)
                    } else {
                        fk.to_ascii_lowercase().cmp(&tk.to_ascii_lowercase())
                    }
                };

                match diff {
                    std::cmp::Ordering::Equal => {
                        // Same key in both: recurse on value
                        let (ref fk, ref fv) = from_pairs[fi];
                        let (_, ref tv) = to_pairs[ti];
                        let new_path = format!("{}/{}", path, encode_pointer_segment(fk));
                        create_patches(patches, &new_path, fv, tv, case_sensitive);
                        fi += 1;
                        ti += 1;
                    }
                    std::cmp::Ordering::Less => {
                        // Key only in `from` → remove
                        let (ref fk, _) = from_pairs[fi];
                        compose_patch(patches, "remove", path, Some(fk), None);
                        fi += 1;
                    }
                    std::cmp::Ordering::Greater => {
                        // Key only in `to` → add
                        let (ref tk, ref tv) = to_pairs[ti];
                        compose_patch(patches, "add", path, Some(tk), Some(tv));
                        ti += 1;
                    }
                }
            }
        }
        // Null == Null (no data to compare) falls here correctly.
        // `Raw` also falls here **deliberately, faithfully reproducing a
        // real bug in upstream** rather than fixing it: cJSON_Utils.c's
        // create_patches has no switch case for cJSON_Raw at all - it
        // silently falls through its `default: break;`, so two Raw values
        // with genuinely different content produce *no* patch. Per this
        // hackathon's Behavioral Equivalence rule ("if C has a bug, the
        // port should have the same bug, not a silent fix"), this port
        // reproduces that exact silent-no-patch behavior instead of
        // correcting it. See BUG_REPORT.md for the standalone writeup of
        // the underlying upstream bug, and DECISIONS.md §6c for the
        // Behavioral Equivalence rationale.
        _ => {}
    }
}

/// Relative-epsilon comparison matching cJSON's `compare_double`
/// (cJSON.c:589-593). Duplicated here rather than importing from value.rs
/// to keep module boundaries clean (same rationale as print.rs's copy).
fn compare_double_patch(a: f64, b: f64) -> bool {
    let max_val = a.abs().max(b.abs());
    (a - b).abs() <= max_val * f64::EPSILON
}

/// Generates a JSON Patch (RFC 6902) document (an array of operations) that
/// transforms `from` into `to`. Mirrors `cJSONUtils_GeneratePatches`
/// (cJSON_Utils.c:1281-1294).
///
/// Returns a `Value::Array` of patch operation objects.
pub fn generate_patches(from: &Value, to: &Value) -> Value {
    let mut patches = Value::array();
    create_patches(&mut patches, "", from, to, false);
    patches
}

/// Case-sensitive variant. Mirrors `cJSONUtils_GeneratePatchesCaseSensitive`.
pub fn generate_patches_case_sensitive(from: &Value, to: &Value) -> Value {
    let mut patches = Value::array();
    create_patches(&mut patches, "", from, to, true);
    patches
}

// ============================================================================
// JSON Merge Patch generation (RFC 7396). Mirrors `generate_merge_patch`
// (cJSON_Utils.c:1391-1471).
// ============================================================================

fn generate_merge_patch_inner(from: &Value, to: &Value, case_sensitive: bool) -> Option<Value> {
    // If either side is not an object, the patch is just a deep copy of `to`
    // (cJSON_Utils.c:1401-1403).
    if !to.is_object() || !from.is_object() {
        return Some(to.duplicate(true));
    }

    // Sort both sides for the merge-walk (mirrors cJSON_Utils.c:1406-1407).
    let mut from_sorted = from.duplicate(true);
    let mut to_sorted = to.duplicate(true);
    sort_object_inner(&mut from_sorted, case_sensitive);
    sort_object_inner(&mut to_sorted, case_sensitive);

    let from_pairs = from_sorted.as_object().unwrap();
    let to_pairs = to_sorted.as_object().unwrap();

    let mut patch = Value::object();
    let mut fi = 0;
    let mut ti = 0;

    while fi < from_pairs.len() || ti < to_pairs.len() {
        let diff = if fi < from_pairs.len() {
            if ti < to_pairs.len() {
                let (ref fk, _) = from_pairs[fi];
                let (ref tk, _) = to_pairs[ti];
                if case_sensitive {
                    fk.cmp(tk)
                } else {
                    fk.to_ascii_lowercase().cmp(&tk.to_ascii_lowercase())
                }
            } else {
                std::cmp::Ordering::Less
            }
        } else {
            std::cmp::Ordering::Greater
        };

        match diff {
            std::cmp::Ordering::Less => {
                // Key only in `from` → patch it to null (delete)
                let (ref fk, _) = from_pairs[fi];
                patch.object_push(fk.clone(), Value::Null).ok();
                fi += 1;
            }
            std::cmp::Ordering::Greater => {
                // Key only in `to` → add it
                let (ref tk, ref tv) = to_pairs[ti];
                patch.object_push(tk.clone(), tv.duplicate(true)).ok();
                ti += 1;
            }
            std::cmp::Ordering::Equal => {
                // Same key in both → recurse if values differ
                let (ref fk, ref fv) = from_pairs[fi];
                let (_, ref tv) = to_pairs[ti];
                if !compare(fv, tv, case_sensitive) {
                    if let Some(sub_patch) = generate_merge_patch_inner(fv, tv, case_sensitive) {
                        patch.object_push(fk.clone(), sub_patch).ok();
                    }
                }
                fi += 1;
                ti += 1;
            }
        }
    }

    // map_or(true, ...) instead of the newer is_none_or: is_none_or was
    // only stabilized in Rust 1.82, and this crate declares no
    // rust-version floor requiring that - see the matching note in
    // print.rs's repeat()/take() fix for the same underlying issue.
    if patch.as_object().map_or(true, |p| p.is_empty()) {
        None
    } else {
        Some(patch)
    }
}

/// Generates an RFC 7396 JSON Merge Patch that transforms `from` into `to`.
/// Returns `None` if the two documents are identical (matching upstream's
/// NULL return for "no patch needed"). Mirrors `cJSONUtils_GenerateMergePatch`
/// (cJSON_Utils.c:1473-1476).
pub fn generate_merge_patch(from: &Value, to: &Value) -> Option<Value> {
    generate_merge_patch_inner(from, to, false)
}

/// Case-sensitive variant. Mirrors `cJSONUtils_GenerateMergePatchCaseSensitive`.
pub fn generate_merge_patch_case_sensitive(from: &Value, to: &Value) -> Option<Value> {
    generate_merge_patch_inner(from, to, true)
}


#[cfg(test)]
mod patch_tests {
    use super::*;
    use crate::parse::parse;
    use crate::print::print_unformatted;

    fn doc(s: &str) -> Value {
        parse(s).unwrap()
    }

    fn apply_and_print(doc_json: &str, patch_json: &str) -> Result<String, PatchError> {
        let mut object = doc(doc_json);
        let patches = doc(patch_json);
        apply_patches(&mut object, &patches)?;
        Ok(print_unformatted(&object).unwrap())
    }

    #[test]
    fn add_replaces_existing_field() {
        let out = apply_and_print(
            r#"{"foo": null}"#,
            r#"[{"op": "add", "path": "/foo", "value": 1}]"#,
        )
        .unwrap();
        assert_eq!(out, r#"{"foo":1}"#);
    }

    #[test]
    fn add_to_array_at_index() {
        let out = apply_and_print(
            r#"["a","b","c"]"#,
            r#"[{"op": "add", "path": "/1", "value": "x"}]"#,
        )
        .unwrap();
        assert_eq!(out, r#"["a","x","b","c"]"#);
    }

    #[test]
    fn add_to_array_append_token() {
        let out = apply_and_print(
            r#"["a","b"]"#,
            r#"[{"op": "add", "path": "/-", "value": "c"}]"#,
        )
        .unwrap();
        assert_eq!(out, r#"["a","b","c"]"#);
    }

    #[test]
    fn add_index_past_end_is_error() {
        let mut object = doc(r#"["a"]"#);
        let patches = doc(r#"[{"op":"add","path":"/5","value":"x"}]"#);
        assert_eq!(apply_patches(&mut object, &patches), Err(PatchError::ArrayIndexInvalid));
    }

    #[test]
    fn remove_object_member() {
        let out = apply_and_print(
            r#"{"foo": "bar", "baz": "qux"}"#,
            r#"[{"op": "remove", "path": "/baz"}]"#,
        )
        .unwrap();
        assert_eq!(out, r#"{"foo":"bar"}"#);
    }

    #[test]
    fn remove_array_element() {
        let out = apply_and_print(
            r#"["a","b","c"]"#,
            r#"[{"op": "remove", "path": "/1"}]"#,
        )
        .unwrap();
        assert_eq!(out, r#"["a","c"]"#);
    }

    #[test]
    fn remove_missing_path_is_error() {
        let mut object = doc(r#"{"foo": "bar"}"#);
        let patches = doc(r#"[{"op":"remove","path":"/nope"}]"#);
        assert_eq!(apply_patches(&mut object, &patches), Err(PatchError::PathNotFound));
    }

    #[test]
    fn replace_object_member() {
        let out = apply_and_print(
            r#"{"foo": "bar"}"#,
            r#"[{"op": "replace", "path": "/foo", "value": "qux"}]"#,
        )
        .unwrap();
        assert_eq!(out, r#"{"foo":"qux"}"#);
    }

    #[test]
    fn move_object_member() {
        let out = apply_and_print(
            r#"{"foo": {"bar": "baz", "waldo": "fred"}, "qux": {"corge": "grault"}}"#,
            r#"[{"op": "move", "from": "/foo/waldo", "path": "/qux/thud"}]"#,
        )
        .unwrap();
        assert_eq!(out, r#"{"foo":{"bar":"baz"},"qux":{"corge":"grault","thud":"fred"}}"#);
    }

    #[test]
    fn move_array_element() {
        let out = apply_and_print(
            r#"["all","grass","cows","eat"]"#,
            r#"[{"op": "move", "from": "/1", "path": "/3"}]"#,
        )
        .unwrap();
        assert_eq!(out, r#"["all","cows","eat","grass"]"#);
    }

    #[test]
    fn copy_object_member() {
        let out = apply_and_print(
            r#"{"foo": {"bar": "baz", "waldo": "fred"}, "qux": {"corge": "grault"}}"#,
            r#"[{"op": "copy", "from": "/foo/waldo", "path": "/qux/thud"}]"#,
        )
        .unwrap();
        assert_eq!(
            out,
            r#"{"foo":{"bar":"baz","waldo":"fred"},"qux":{"corge":"grault","thud":"fred"}}"#
        );
    }

    #[test]
    fn test_op_success_and_failure() {
        let mut object = doc(r#"{"baz": "qux", "foo": ["a", 2, "c"]}"#);
        let ok_patch = doc(r#"[{"op": "test", "path": "/baz", "value": "qux"}]"#);
        assert_eq!(apply_patches(&mut object, &ok_patch), Ok(()));

        let bad_patch = doc(r#"[{"op": "test", "path": "/baz", "value": "bar"}]"#);
        assert_eq!(apply_patches(&mut object, &bad_patch), Err(PatchError::TestFailed));
    }

    #[test]
    fn add_replaces_root_document() {
        let out = apply_and_print(r#"{}"#, r#"[{"op": "add", "path": "", "value": []}]"#).unwrap();
        assert_eq!(out, "[]");
    }

    #[test]
    fn empty_patch_list_is_a_no_op() {
        let out = apply_and_print(r#"{"foo": 1}"#, "[]").unwrap();
        assert_eq!(out, r#"{"foo":1}"#);
    }

    #[test]
    fn sequential_patches_apply_in_order() {
        let out = apply_and_print(
            r#"{"foo": 1}"#,
            r#"[
                {"op": "add", "path": "/bar", "value": 2},
                {"op": "replace", "path": "/foo", "value": 10},
                {"op": "remove", "path": "/bar"}
            ]"#,
        )
        .unwrap();
        assert_eq!(out, r#"{"foo":10}"#);
    }

    #[test]
    fn non_transactional_partial_application_on_failure() {
        // Mirrors upstream: already-applied ops before a failing one stay
        // applied (no rollback).
        let mut object = doc(r#"{"foo": 1}"#);
        let patches = doc(
            r#"[
                {"op": "add", "path": "/bar", "value": 2},
                {"op": "remove", "path": "/does-not-exist"}
            ]"#,
        );
        assert_eq!(apply_patches(&mut object, &patches), Err(PatchError::PathNotFound));
        assert_eq!(print_unformatted(&object).unwrap(), r#"{"foo":1,"bar":2}"#);
    }

    #[test]
    fn malformed_patches_document_is_rejected() {
        let mut object = doc(r#"{}"#);
        let not_an_array = doc(r#"{"op":"add"}"#);
        assert_eq!(apply_patches(&mut object, &not_an_array), Err(PatchError::MalformedPatch));
    }

    // --- Merge Patch (RFC 7396 §1 example, verbatim) ---

    #[test]
    fn merge_patch_rfc7396_example() {
        let target = doc(
            r#"{
                "title": "Goodbye!",
                "author": {"givenName": "John", "familyName": "Doe"},
                "tags": ["example", "sample"],
                "content": "This will be unchanged"
            }"#,
        );
        let patch = doc(
            r#"{
                "title": "Hello!",
                "phoneNumber": "+01-123-456-7890",
                "author": {"familyName": null},
                "tags": ["example"]
            }"#,
        );
        let merged = merge_patch(target, &patch);
        // Note on key order: every key the patch touches is detached then
        // re-appended (mirrors upstream's DetachItemFromObject +
        // AddItemToObject), so touched keys move to the end in the order
        // they appear in the *patch*, not the target. This matches the
        // original C implementation's actual observable behavior, not just
        // RFC 7396's value semantics (the RFC itself doesn't mandate order).
        assert_eq!(
            print_unformatted(&merged).unwrap(),
            r#"{"content":"This will be unchanged","title":"Hello!","phoneNumber":"+01-123-456-7890","author":{"givenName":"John"},"tags":["example"]}"#
        );
    }

    #[test]
    fn merge_patch_scalar_patch_replaces_wholesale() {
        let target = doc(r#"{"a":"b"}"#);
        let patch = doc(r#""just a string""#);
        let merged = merge_patch(target, &patch);
        assert_eq!(merged, Value::string("just a string"));
    }

    #[test]
    fn merge_patch_array_patch_replaces_wholesale() {
        let target = doc(r#"{"a":"b"}"#);
        let patch = doc(r#"["c"]"#);
        let merged = merge_patch(target, &patch);
        assert_eq!(merged, Value::Array(vec![Value::string("c")]));
    }

    #[test]
    fn merge_patch_null_deletes_key() {
        let target = doc(r#"{"a":"b","b":"c"}"#);
        let patch = doc(r#"{"a":null}"#);
        let merged = merge_patch(target, &patch);
        assert_eq!(print_unformatted(&merged).unwrap(), r#"{"b":"c"}"#);
    }

    #[test]
    fn merge_patch_creates_object_when_target_is_not_one() {
        let target = doc(r#"["a","b"]"#);
        let patch = doc(r#"{"a":"b"}"#);
        let merged = merge_patch(target, &patch);
        assert_eq!(print_unformatted(&merged).unwrap(), r#"{"a":"b"}"#);
    }

    // --- sort_object ---

    #[test]
    fn sort_object_sorts_keys_case_insensitively() {
        let mut obj = doc(r#"{"c":3,"a":1,"B":2}"#);
        sort_object(&mut obj);
        assert_eq!(print_unformatted(&obj).unwrap(), r#"{"a":1,"B":2,"c":3}"#);
    }

    #[test]
    fn sort_object_case_sensitive_uses_ascii_order() {
        let mut obj = doc(r#"{"c":3,"a":1,"B":2}"#);
        sort_object_case_sensitive(&mut obj);
        // ASCII order: 'B'(66) < 'a'(97) < 'c'(99)
        assert_eq!(print_unformatted(&obj).unwrap(), r#"{"B":2,"a":1,"c":3}"#);
    }

    #[test]
    fn sort_object_no_op_on_non_objects() {
        let mut arr = doc(r#"[3,1,2]"#);
        sort_object(&mut arr); // should not panic
        assert_eq!(print_unformatted(&arr).unwrap(), "[3,1,2]");
    }

    #[test]
    fn sort_object_stable_preserves_duplicate_key_order() {
        // Two entries with key "a" — stable sort preserves their relative order.
        let mut obj = Value::Object(vec![
            ("a".to_string(), Value::number(1.0)),
            ("a".to_string(), Value::number(2.0)),
        ]);
        sort_object(&mut obj);
        let pairs = obj.as_object().unwrap();
        assert_eq!(pairs[0].1.as_f64(), Some(1.0));
        assert_eq!(pairs[1].1.as_f64(), Some(2.0));
    }

    // --- generate_patches (RFC 6902 diff) ---

    #[test]
    fn generate_patches_identical_documents_produces_empty_patch() {
        let a = doc(r#"{"a":1,"b":"hello"}"#);
        let b = doc(r#"{"a":1,"b":"hello"}"#);
        let patches = generate_patches(&a, &b);
        assert_eq!(patches.as_array().unwrap().len(), 0);
    }

    #[test]
    fn generate_patches_replace_scalar() {
        let a = doc(r#"{"a":1}"#);
        let b = doc(r#"{"a":2}"#);
        let patches = generate_patches(&a, &b);
        assert_eq!(
            print_unformatted(&patches).unwrap(),
            r#"[{"op":"replace","path":"/a","value":2}]"#
        );
    }

    #[test]
    fn generate_patches_add_key() {
        let a = doc(r#"{"a":1}"#);
        let b = doc(r#"{"a":1,"b":2}"#);
        let patches = generate_patches(&a, &b);
        assert_eq!(
            print_unformatted(&patches).unwrap(),
            r#"[{"op":"add","path":"/b","value":2}]"#
        );
    }

    #[test]
    fn generate_patches_remove_key() {
        let a = doc(r#"{"a":1,"b":2}"#);
        let b = doc(r#"{"a":1}"#);
        let patches = generate_patches(&a, &b);
        assert_eq!(
            print_unformatted(&patches).unwrap(),
            r#"[{"op":"remove","path":"/b"}]"#
        );
    }

    #[test]
    fn generate_patches_array_add_and_remove() {
        let a = doc(r#"["a","b","c"]"#);
        let b = doc(r#"["a","b","c","d"]"#);
        let patches = generate_patches(&a, &b);
        assert_eq!(
            print_unformatted(&patches).unwrap(),
            r#"[{"op":"add","path":"/-","value":"d"}]"#
        );

        let a2 = doc(r#"["a","b","c"]"#);
        let b2 = doc(r#"["a","b"]"#);
        let patches2 = generate_patches(&a2, &b2);
        assert_eq!(
            print_unformatted(&patches2).unwrap(),
            r#"[{"op":"remove","path":"/2"}]"#
        );
    }

    #[test]
    fn generate_patches_array_element_changed() {
        let a = doc(r#"[1,2,3]"#);
        let b = doc(r#"[1,99,3]"#);
        let patches = generate_patches(&a, &b);
        assert_eq!(
            print_unformatted(&patches).unwrap(),
            r#"[{"op":"replace","path":"/1","value":99}]"#
        );
    }

    #[test]
    fn generate_patches_type_change_produces_replace() {
        let a = doc(r#"{"a":1}"#);
        let b = doc(r#"{"a":"string"}"#);
        let patches = generate_patches(&a, &b);
        assert_eq!(
            print_unformatted(&patches).unwrap(),
            r#"[{"op":"replace","path":"/a","value":"string"}]"#
        );
    }

    #[test]
    // Faithfully reproduces a real upstream bug (see BUG_REPORT.md):
    // cJSON_Utils.c's create_patches has no switch case for cJSON_Raw, so
    // it silently falls through `default: break;` and produces NO patch
    // even when two Raw values have genuinely different content. This is
    // a deliberate deviation from "correct" diffing behavior, kept
    // intentionally to match upstream's actual output for Behavioral
    // Equivalence, per this hackathon's rule that a bug in the original
    // must be reproduced, not silently fixed.
    fn generate_patches_raw_content_change_produces_no_patch_matching_upstream_bug() {
        let mut a = Value::object();
        a.object_push("r", Value::raw("[1,2]")).unwrap();
        let mut b = Value::object();
        b.object_push("r", Value::raw("[1,2,3]")).unwrap();

        let patches = generate_patches(&a, &b);
        assert_eq!(
            patches,
            Value::array(),
            "Raw content differs but upstream's create_patches has no case for \
             cJSON_Raw, so it must silently produce no patch, matching C exactly"
        );
    }

    #[test]
    fn generate_patches_nested_objects() {
        let a = doc(r#"{"x":{"a":1,"b":2}}"#);
        let b = doc(r#"{"x":{"a":1,"b":3}}"#);
        let patches = generate_patches(&a, &b);
        assert_eq!(
            print_unformatted(&patches).unwrap(),
            r#"[{"op":"replace","path":"/x/b","value":3}]"#
        );
    }

    #[test]
    fn generate_patches_roundtrip_apply() {
        // Generate patches, then apply them — result should equal `to`.
        let from = doc(r#"{"a":1,"b":[1,2,3],"c":{"nested":true}}"#);
        let to = doc(r#"{"a":2,"b":[1,2],"d":"new"}"#);
        let patches = generate_patches(&from, &to);

        let mut result = from.duplicate(true);
        apply_patches(&mut result, &patches).unwrap();
        assert!(compare(&result, &to, true));
    }

    #[test]
    fn generate_patches_escapes_keys_with_tilde_and_slash() {
        let a = doc(r#"{"a/b":1,"m~n":2}"#);
        let b = doc(r#"{"a/b":10,"m~n":20}"#);
        let patches = generate_patches(&a, &b);
        let patches_str = print_unformatted(&patches).unwrap();
        // Keys should be escaped: a/b → a~1b, m~n → m~0n
        assert!(patches_str.contains("a~1b"));
        assert!(patches_str.contains("m~0n"));
    }

    // --- add_patch_to_array ---

    #[test]
    fn add_patch_to_array_builds_correct_structure() {
        let mut arr = Value::array();
        add_patch_to_array(&mut arr, "add", "/foo", Some(&Value::number(42.0)));
        assert_eq!(
            print_unformatted(&arr).unwrap(),
            r#"[{"op":"add","path":"/foo","value":42}]"#
        );
    }

    #[test]
    fn add_patch_to_array_without_value() {
        let mut arr = Value::array();
        add_patch_to_array(&mut arr, "remove", "/foo", None);
        assert_eq!(
            print_unformatted(&arr).unwrap(),
            r#"[{"op":"remove","path":"/foo"}]"#
        );
    }

    // --- generate_merge_patch (RFC 7396 diff) ---

    #[test]
    fn generate_merge_patch_identical_returns_none() {
        let a = doc(r#"{"a":1,"b":"hello"}"#);
        let b = doc(r#"{"a":1,"b":"hello"}"#);
        assert_eq!(generate_merge_patch(&a, &b), None);
    }

    #[test]
    fn generate_merge_patch_changed_value() {
        let a = doc(r#"{"a":1,"b":2}"#);
        let b = doc(r#"{"a":1,"b":3}"#);
        let patch = generate_merge_patch(&a, &b).unwrap();
        assert_eq!(print_unformatted(&patch).unwrap(), r#"{"b":3}"#);
    }

    #[test]
    fn generate_merge_patch_added_key() {
        let a = doc(r#"{"a":1}"#);
        let b = doc(r#"{"a":1,"b":2}"#);
        let patch = generate_merge_patch(&a, &b).unwrap();
        assert_eq!(print_unformatted(&patch).unwrap(), r#"{"b":2}"#);
    }

    #[test]
    fn generate_merge_patch_removed_key() {
        let a = doc(r#"{"a":1,"b":2}"#);
        let b = doc(r#"{"a":1}"#);
        let patch = generate_merge_patch(&a, &b).unwrap();
        assert_eq!(print_unformatted(&patch).unwrap(), r#"{"b":null}"#);
    }

    #[test]
    fn generate_merge_patch_type_change_returns_whole_to() {
        let a = doc(r#"{"a":1}"#);
        let b = doc(r#"[1,2,3]"#);
        let patch = generate_merge_patch(&a, &b).unwrap();
        assert_eq!(print_unformatted(&patch).unwrap(), r#"[1,2,3]"#);
    }

    #[test]
    fn generate_merge_patch_roundtrip() {
        let from = doc(r#"{"title":"Goodbye!","author":{"givenName":"John","familyName":"Doe"},"tags":["example","sample"],"content":"unchanged"}"#);
        let to = doc(r#"{"title":"Hello!","author":{"givenName":"John"},"tags":["example"],"content":"unchanged","phoneNumber":"+01-123"}"#);
        let patch = generate_merge_patch(&from, &to).unwrap();
        let result = merge_patch(from, &patch);
        assert!(compare(&result, &to, true));
    }

    #[test]
    fn generate_merge_patch_nested_object() {
        let a = doc(r#"{"x":{"a":1,"b":2}}"#);
        let b = doc(r#"{"x":{"a":1,"b":3}}"#);
        let patch = generate_merge_patch(&a, &b).unwrap();
        assert_eq!(print_unformatted(&patch).unwrap(), r#"{"x":{"b":3}}"#);
    }
}

