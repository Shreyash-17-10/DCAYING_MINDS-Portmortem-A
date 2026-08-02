//! JSON Pointer / JSON Patch / JSON Merge Patch (RFC 6901 / RFC 6902 / RFC 7396).
//! Source: cJSON_Utils.c / cJSON_Utils.h.
//!
//! This module contains JSON Pointer (RFC 6901), JSON Patch (RFC 6902) apply,
//! and JSON Merge Patch (RFC 7396) apply.
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
//!   `compare_pointers`) rather than via in-place byte mutation.

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
fn get_item_from_pointer<'a>(
    object: &'a Value,
    pointer: &str,
    case_sensitive: bool,
) -> Option<&'a Value> {
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
            Value::Object(pairs) => pairs
                .iter()
                .find(|(k, _)| compare_pointer_segment(k, segment, case_sensitive))
                .map(|(_, v)| v)?,
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
        index = index
            .checked_mul(10)?
            .checked_add((bytes[pos] - b'0') as usize)?;
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
            let ok = (escaped == Some('0') && name[ni] == '~')
                || (escaped == Some('1') && name[ni] == '/');
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
        assert_eq!(
            find_pointer_from_object_to(&root, &root),
            Some(String::new())
        );
    }

    #[test]
    fn finds_pointer_to_array_element() {
        let root = Value::Object(vec![(
            "numbers".to_string(),
            Value::Array((0..10).map(|n| Value::number(n as f64)).collect()),
        )]);
        let numbers = root.object_get("numbers").unwrap();
        let six = numbers.array_get(6).unwrap();
        assert_eq!(
            find_pointer_from_object_to(&root, numbers),
            Some("/numbers".to_string())
        );
        assert_eq!(
            find_pointer_from_object_to(&root, six),
            Some("/numbers/6".to_string())
        );
    }

    #[test]
    fn finds_pointer_and_escapes_tilde_and_slash_in_keys() {
        let obj1 = Value::Object(vec![("m~n".to_string(), Value::string("m~n"))]);
        let target1 = obj1.object_get("m~n").unwrap();
        assert_eq!(
            find_pointer_from_object_to(&obj1, target1),
            Some("/m~0n".to_string())
        );

        let obj2 = Value::Object(vec![("m/n".to_string(), Value::string("m/n"))]);
        let target2 = obj2.object_get("m/n").unwrap();
        assert_eq!(
            find_pointer_from_object_to(&obj2, target2),
            Some("/m~1n".to_string())
        );
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
fn get_pointer_mut<'a>(
    value: &'a mut Value,
    pointer: &str,
    case_sensitive: bool,
) -> Option<&'a mut Value> {
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
fn apply_patch_inner(
    object: &mut Value,
    patch: &Value,
    case_sensitive: bool,
) -> Result<(), PatchError> {
    let path = get_object_item(patch, "path", case_sensitive)
        .and_then(Value::as_str)
        .ok_or(PatchError::MalformedPatch)?;

    let opcode =
        decode_patch_operation(patch, case_sensitive).ok_or(PatchError::InvalidOperation)?;

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
            let v =
                get_object_item(patch, "value", case_sensitive).ok_or(PatchError::MissingValue)?;
            v.duplicate(true)
        }
        PatchOp::Remove | PatchOp::Test => unreachable!("handled above"),
    };

    // Add "value" at "path" (cJSON_Utils.c:959-1023).
    let (parent_pointer, child_segment) =
        split_last_pointer_segment(path).ok_or(PatchError::PathNotFound)?;
    let parent =
        get_pointer_mut(object, parent_pointer, case_sensitive).ok_or(PatchError::PathNotFound)?;

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

fn apply_patches_inner(
    object: &mut Value,
    patches: &Value,
    case_sensitive: bool,
) -> Result<(), PatchError> {
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

    let mut target = if target.is_object() {
        target
    } else {
        Value::object()
    };

for (key, patch_child) in patch_pairs.iter().cloned() {
        if patch_child.is_null() {
            // RFC 7396: null in the patch means "delete this key".
            target.object_delete(key, case_sensitive);
        } else {
            let existing = target
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
        assert_eq!(
            apply_patches(&mut object, &patches),
            Err(PatchError::ArrayIndexInvalid)
        );
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
        let out =
            apply_and_print(r#"["a","b","c"]"#, r#"[{"op": "remove", "path": "/1"}]"#).unwrap();
        assert_eq!(out, r#"["a","c"]"#);
    }

    #[test]
    fn remove_missing_path_is_error() {
        let mut object = doc(r#"{"foo": "bar"}"#);
        let patches = doc(r#"[{"op":"remove","path":"/nope"}]"#);
        assert_eq!(
            apply_patches(&mut object, &patches),
            Err(PatchError::PathNotFound)
        );
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
        assert_eq!(
            out,
            r#"{"foo":{"bar":"baz"},"qux":{"corge":"grault","thud":"fred"}}"#
        );
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
        assert_eq!(
            apply_patches(&mut object, &bad_patch),
            Err(PatchError::TestFailed)
        );
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
        let patches = doc(r#"[
                {"op": "add", "path": "/bar", "value": 2},
                {"op": "remove", "path": "/does-not-exist"}
            ]"#);
        assert_eq!(
            apply_patches(&mut object, &patches),
            Err(PatchError::PathNotFound)
        );
        assert_eq!(print_unformatted(&object).unwrap(), r#"{"foo":1,"bar":2}"#);
    }

    #[test]
    fn malformed_patches_document_is_rejected() {
        let mut object = doc(r#"{}"#);
        let not_an_array = doc(r#"{"op":"add"}"#);
        assert_eq!(
            apply_patches(&mut object, &not_an_array),
            Err(PatchError::MalformedPatch)
        );
    }

    // --- Merge Patch (RFC 7396 §1 example, verbatim) ---

    #[test]
    fn merge_patch_rfc7396_example() {
        let target = doc(r#"{
                "title": "Goodbye!",
                "author": {"givenName": "John", "familyName": "Doe"},
                "tags": ["example", "sample"],
                "content": "This will be unchanged"
            }"#);
        let patch = doc(r#"{
                "title": "Hello!",
                "phoneNumber": "+01-123-456-7890",
                "author": {"familyName": null},
                "tags": ["example"]
            }"#);
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
}
