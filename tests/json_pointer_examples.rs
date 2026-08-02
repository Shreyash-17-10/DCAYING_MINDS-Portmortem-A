//! Ported from cJSON's tests/old_utils_tests.c: `json_pointer_tests`.
//!
//! This is the RFC 6901 conformance case built into cJSON's own test suite
//! (the exact document and pointer expressions are reproduced verbatim from
//! upstream, including all of RFC 6901 Appendix A's tricky key names: empty
//! string key, "a/b", "c%d", "e^f", "g|h", "i\\j", "k\"l", " ", "m~n").
//!
//! Deviation from the original, documented: the C test asserts *pointer
//! identity* (`TEST_ASSERT_EQUAL_PTR`) between `cJSONUtils_GetPointer(...)`
//! and `cJSON_GetObjectItem(...)`, because both walk the same in-memory
//! tree and must land on the exact same node. Our `Value` tree is owned
//! data without a shared-identity concept exposed at this layer, so each
//! assertion below is rewritten as a *value-equality* check against the
//! same lookup performed independently via `object_get`/`array_get`. This
//! preserves the test's intent (does GetPointer resolve to the same node
//! GetObjectItem would?) without requiring pointer identity.

use cjson_rs::parse::parse;
use cjson_rs::utils::get_pointer;

#[test]
fn json_pointer_resolves_rfc6901_examples() {
    let json = r#"{
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
    }"#;

    let root = parse(json).expect("fixture JSON must parse");

    // GetPointer(root, "") == root
    assert_eq!(get_pointer(&root, ""), Some(&root));

    // GetPointer(root, "/foo") == GetObjectItem(root, "foo")
    assert_eq!(get_pointer(&root, "/foo"), root.object_get("foo"));

    // GetPointer(root, "/foo/0") == GetObjectItem(root, "foo")->child (first element)
    let foo_array = root.object_get("foo").expect("\"foo\" key must exist");
    assert_eq!(get_pointer(&root, "/foo/0"), foo_array.array_get(0));

    // GetPointer(root, "/") == GetObjectItem(root, "") -- the empty-string key
    assert_eq!(get_pointer(&root, "/"), root.object_get(""));

    // ~1 decodes to '/'
    assert_eq!(get_pointer(&root, "/a~1b"), root.object_get("a/b"));

    // These characters need no escaping in JSON Pointer.
    assert_eq!(get_pointer(&root, "/c%d"), root.object_get("c%d"));
    assert_eq!(get_pointer(&root, "/e^f"), root.object_get("e^f"));
    assert_eq!(get_pointer(&root, "/g|h"), root.object_get("g|h"));
    assert_eq!(get_pointer(&root, "/i\\j"), root.object_get("i\\j"));
    assert_eq!(get_pointer(&root, "/k\"l"), root.object_get("k\"l"));
    assert_eq!(get_pointer(&root, "/ "), root.object_get(" "));

    // ~0 decodes to '~'
    assert_eq!(get_pointer(&root, "/m~0n"), root.object_get("m~n"));
}
