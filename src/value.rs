//! Core JSON value type.
//! Replaces C's `struct cJSON` (intrusive linked-list tree, raw pointers, int type tag)
//! from cJSON.h with an owned, memory-safe tree. No `unsafe` used.
//!
//! Design notes (see DECISIONS.md for full rationale):
//! - C's 7 type tags (False/True/NULL/Number/String/Array/Object/Raw) map to `Value` variants.
//!   True/False collapse into `Bool(bool)`.
//! - C's `next/prev` sibling pointers + `child` pointer become `Vec<Value>` (Array) and
//!   `Vec<(String, Value)>` (Object) - owned, drop-safe, order-preserving.
//! - `valuestring`/`string` (char*) become `String`. `valueint` (deprecated in upstream,
//!   derived from valuedouble) becomes a method, not stored state - see `Value::as_int`.
//! - `cJSON_IsReference` / `cJSON_StringIsConst` (ownership/const flags on the C struct)
//!   are moot: Rust ownership is enforced by the type system.

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    /// Raw, pre-formatted JSON emitted verbatim on print (cJSON_Raw).
    Raw(String),
    Array(Vec<Value>),
    /// Ordered key-value pairs, matches cJSON's object-as-linked-list semantics.
    Object(Vec<(String, Value)>),
}

impl Value {
    // --- Constructors (mirror cJSON_Create*) ---

    pub fn null() -> Self {
        Value::Null
    }
    pub fn boolean(b: bool) -> Self {
        Value::Bool(b)
    }
    pub fn number(n: f64) -> Self {
        Value::Number(n)
    }
    pub fn string<S: Into<String>>(s: S) -> Self {
        Value::String(s.into())
    }
    pub fn raw<S: Into<String>>(s: S) -> Self {
        Value::Raw(s.into())
    }
    pub fn array() -> Self {
        Value::Array(Vec::new())
    }
    pub fn object() -> Self {
        Value::Object(Vec::new())
    }

    // --- Type predicates (mirror cJSON_Is*) ---

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    pub fn is_bool(&self) -> bool {
        matches!(self, Value::Bool(_))
    }
    pub fn is_true(&self) -> bool {
        matches!(self, Value::Bool(true))
    }
    pub fn is_false(&self) -> bool {
        matches!(self, Value::Bool(false))
    }
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Number(_))
    }
    pub fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }
    pub fn is_raw(&self) -> bool {
        matches!(self, Value::Raw(_))
    }
    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }

    // --- Accessors ---

    /// Mirrors the deprecated `valueint` field: truncates toward zero,
    /// matching cJSON's `(int)number` cast behavior.
    pub fn as_int(&self) -> Option<i32> {
        match self {
            Value::Number(n) => Some(*n as i32),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) | Value::Raw(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(pairs) => Some(pairs),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut Vec<(String, Value)>> {
        match self {
            Value::Object(pairs) => Some(pairs),
            _ => None,
        }
    }

    // --- Array mutation (mirror cJSON_AddItemToArray) ---

    /// Returns Err(item) unchanged if `self` is not an Array (mirrors cJSON's
    /// `false` return from cJSON_AddItemToArray on type mismatch, instead of
    /// silently corrupting state).
    pub fn array_push(&mut self, item: Value) -> Result<(), Value> {
        match self {
            Value::Array(items) => {
                items.push(item);
                Ok(())
            }
            _ => Err(item),
        }
    }

    pub fn array_len(&self) -> Option<usize> {
        self.as_array().map(|a| a.len())
    }

    pub fn array_get(&self, index: usize) -> Option<&Value> {
        self.as_array().and_then(|a| a.get(index))
    }

    // --- Object mutation (mirror cJSON_AddItemToObject / GetObjectItem) ---

    /// Mirrors cJSON_AddItemToObject: does not deduplicate existing keys,
    /// matching upstream's append-only linked-list behavior (duplicate keys
    /// allowed; lookup returns the first match, same as C).
    pub fn object_push<S: Into<String>>(&mut self, key: S, item: Value) -> Result<(), Value> {
        match self {
            Value::Object(pairs) => {
                pairs.push((key.into(), item));
                Ok(())
            }
            _ => Err(item),
        }
    }

    /// Case-sensitive lookup, mirrors cJSON_GetObjectItemCaseSensitive.
    pub fn object_get(&self, key: &str) -> Option<&Value> {
        self.as_object()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// Case-insensitive lookup, mirrors cJSON_GetObjectItem.
    pub fn object_get_ci(&self, key: &str) -> Option<&Value> {
        self.as_object()?
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    pub fn object_has(&self, key: &str) -> bool {
        self.object_get(key).is_some()
    }

    // ========================================================================
    // Phase 5: manipulation API (cJSON.c Detach/Delete/Insert/Replace/
    // Duplicate, and the AddXToObject convenience constructors).
    // ========================================================================

    // --- Detach / Delete (mirror cJSON_Detach/DeleteItemFromArray) ---

    /// Removes and returns the item at `index`, or `None` if out of bounds.
    /// Mirrors `cJSON_DetachItemFromArray` (which returns NULL for a
    /// negative or out-of-range index rather than for any other reason -
    /// negative indices are excluded here simply by `index` being `usize`).
    pub fn array_detach(&mut self, index: usize) -> Option<Value> {
        let items = self.as_array_mut()?;
        if index >= items.len() {
            return None;
        }
        Some(items.remove(index))
    }

    /// Removes and drops the item at `index`. Mirrors
    /// `cJSON_DeleteItemFromArray` (detach + delete).
    pub fn array_delete(&mut self, index: usize) {
        self.array_detach(index);
    }

    /// Removes and returns the first item whose key matches (mirrors
    /// `cJSON_DetachItemFromObject[CaseSensitive]`).
    pub fn object_detach(&mut self, key: &str, case_sensitive: bool) -> Option<Value> {
        let pairs = self.as_object_mut()?;
        let idx = pairs.iter().position(|(k, _)| {
            if case_sensitive {
                k == key
            } else {
                k.eq_ignore_ascii_case(key)
            }
        })?;
        Some(pairs.remove(idx).1)
    }

    /// Mirrors `cJSON_DeleteItemFromObject[CaseSensitive]`.
    pub fn object_delete(&mut self, key: &str, case_sensitive: bool) {
        self.object_detach(key, case_sensitive);
    }

    // --- Insert / Replace (mirror cJSON_InsertItemInArray / ReplaceItemIn*) ---

    /// Inserts `item` at `index`, shifting later items right. If `index` is
    /// past the end, appends instead - mirrors `cJSON_InsertItemInArray`'s
    /// fallback to `add_item_to_array` when `get_array_item` finds nothing
    /// at `which`. Returns `Err(item)` unchanged if `self` isn't an Array.
    pub fn array_insert(&mut self, index: usize, item: Value) -> Result<(), Value> {
        match self {
            Value::Array(items) => {
                let at = index.min(items.len());
                items.insert(at, item);
                Ok(())
            }
            _ => Err(item),
        }
    }

    /// Replaces the item at `index` in place. Returns `false` if `self`
    /// isn't an Array or `index` is out of bounds - mirrors
    /// `cJSON_ReplaceItemInArray` returning `false` when
    /// `get_array_item` can't find `which`.
    pub fn array_replace(&mut self, index: usize, replacement: Value) -> bool {
        match self.as_array_mut() {
            Some(items) if index < items.len() => {
                items[index] = replacement;
                true
            }
            _ => false,
        }
    }

    /// Replaces the value stored at the first matching key, **renaming it to
    /// `key`** in the process (mirrors `replace_item_in_object`, which
    /// overwrites `replacement->string` with the search string regardless of
    /// what key was actually matched - relevant when `case_sensitive` is
    /// false and the existing key's casing differs from `key`). Returns
    /// `false` if `self` isn't an Object or no matching key exists.
    pub fn object_replace(&mut self, key: &str, case_sensitive: bool, replacement: Value) -> bool {
        match self.as_object_mut() {
            Some(pairs) => {
                let idx = pairs.iter().position(|(k, _)| {
                    if case_sensitive {
                        k == key
                    } else {
                        k.eq_ignore_ascii_case(key)
                    }
                });
                match idx {
                    Some(i) => {
                        pairs[i] = (key.to_string(), replacement);
                        true
                    }
                    None => false,
                }
            }
            None => false,
        }
    }

    // --- Duplicate (mirrors cJSON_Duplicate) ---

    /// Deep- or shallow-copies `self`. Mirrors `cJSON_Duplicate`: with
    /// `recurse = false`, a container (Array/Object) is duplicated as an
    /// **empty** container of the same kind - children are only copied when
    /// `recurse` is true. (Rust's ownership model makes this less useful
    /// than in C, where `recurse = false` exists mainly to make a
    /// standalone node before manually re-parenting it, but the behavior is
    /// preserved for equivalence.)
    pub fn duplicate(&self, recurse: bool) -> Value {
        match self {
            Value::Array(items) => {
                if recurse {
                    Value::Array(items.iter().map(|v| v.duplicate(true)).collect())
                } else {
                    Value::Array(Vec::new())
                }
            }
            Value::Object(pairs) => {
                if recurse {
                    Value::Object(
                        pairs
                            .iter()
                            .map(|(k, v)| (k.clone(), v.duplicate(true)))
                            .collect(),
                    )
                } else {
                    Value::Object(Vec::new())
                }
            }
            scalar => scalar.clone(),
        }
    }

    // --- AddXToObject convenience constructors (mirror cJSON_AddXToObject) ---
    // Each pushes a freshly constructed value under `name` and returns
    // whether it succeeded (fails only if `self` isn't an Object, matching
    // `add_item_to_object`'s failure mode - our push functions can't fail
    // for allocation reasons the way C's can).

    pub fn add_null_to_object<S: Into<String>>(&mut self, name: S) -> bool {
        self.object_push(name, Value::null()).is_ok()
    }
    pub fn add_bool_to_object<S: Into<String>>(&mut self, name: S, b: bool) -> bool {
        self.object_push(name, Value::boolean(b)).is_ok()
    }
    pub fn add_number_to_object<S: Into<String>>(&mut self, name: S, n: f64) -> bool {
        self.object_push(name, Value::number(n)).is_ok()
    }
    pub fn add_string_to_object<S: Into<String>, V: Into<String>>(
        &mut self,
        name: S,
        s: V,
    ) -> bool {
        self.object_push(name, Value::string(s)).is_ok()
    }
    pub fn add_raw_to_object<S: Into<String>, V: Into<String>>(&mut self, name: S, raw: V) -> bool {
        self.object_push(name, Value::raw(raw)).is_ok()
    }
    pub fn add_array_to_object<S: Into<String>>(&mut self, name: S) -> bool {
        self.object_push(name, Value::array()).is_ok()
    }
    pub fn add_object_to_object<S: Into<String>>(&mut self, name: S) -> bool {
        self.object_push(name, Value::object()).is_ok()
    }
}

// ============================================================================
// Phase 5: cJSON_Compare. A separate free function (not PartialEq) because
// the semantics differ from derived structural equality in three ways:
// - Numbers compare with a relative-epsilon tolerance (`compare_double`),
//   not bit-exact equality.
// - Object key order doesn't matter (each side's keys are looked up in the
//   other), unlike derived PartialEq on `Vec<(String, Value)>` which is
//   order-sensitive.
// - Object key lookup can be case-insensitive, controlled by `case_sensitive`.
// ============================================================================

/// Mirrors `cJSON_Compare` (cJSON.c:3072-3175).
pub fn compare(a: &Value, b: &Value, case_sensitive: bool) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => compare_double(*x, *y),
        // Raw is compared the same way as String: exact byte-for-byte
        // equality of the stored text, mirroring cJSON.c's shared
        // cJSON_String/cJSON_Raw case (a `strcmp`, no JSON-aware parsing).
        (Value::String(x), Value::String(y)) | (Value::Raw(x), Value::Raw(y)) => x == y,
        (Value::Array(xs), Value::Array(ys)) => {
            xs.len() == ys.len()
                && xs
                    .iter()
                    .zip(ys.iter())
                    .all(|(x, y)| compare(x, y, case_sensitive))
        }
        (Value::Object(xs), Value::Object(ys)) => {
            // Checked in both directions (mirrors upstream's own two-pass
            // approach, needed because key lookup only finds the *first*
            // match): every key in `xs` must exist in `ys` with an equal
            // value, and vice versa.
            xs.iter().all(|(k, v)| {
                object_lookup(ys, k, case_sensitive)
                    .is_some_and(|bv| compare(v, bv, case_sensitive))
            }) && ys.iter().all(|(k, v)| {
                object_lookup(xs, k, case_sensitive)
                    .is_some_and(|av| compare(v, av, case_sensitive))
            })
        }
        _ => false,
    }
}

/// Case-(in)sensitive first-match key lookup, shared by both directions of
/// `compare`'s object comparison. Pulled out as a plain function rather than
/// a closure: a closure taking two independently-lifetimed `&` parameters
/// and returning a reference tied to only one of them needs a higher-ranked
/// lifetime bound that Rust's closure inference can't derive on its own
/// (it picks a single unified lifetime and then rejects the return type) -
/// a `fn` states the two lifetimes explicitly instead.
fn object_lookup<'a>(
    pairs: &'a [(String, Value)],
    key: &str,
    case_sensitive: bool,
) -> Option<&'a Value> {
    pairs
        .iter()
        .find(|(k, _)| {
            if case_sensitive {
                k == key
            } else {
                k.eq_ignore_ascii_case(key)
            }
        })
        .map(|(_, v)| v)
}

/// Mirrors compare_double (cJSON.c:589-593): relative-epsilon comparison,
/// shared with print.rs's number-round-trip check (kept as a private
/// duplicate here rather than adding a cross-module dependency for one
/// three-line function - see DECISIONS.md).
fn compare_double(a: f64, b: f64) -> bool {
    let max_val = a.abs().max(b.abs());
    (a - b).abs() <= max_val * f64::EPSILON
}

// ============================================================================
// Display and FromStr: Rust trait implementations.
// `Display` enables `format!("{}", value)` for compact output and
// `format!("{:#}", value)` for pretty-printed output.
// `FromStr` enables `let v: Value = json_str.parse()?`.
// Neither of these has a C equivalent — they leverage Rust's trait system
// to provide a more ergonomic API than cJSON's function-based interface.
// ============================================================================

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Use the alternate flag (`{:#}`) for pretty-printed output.
        let result = if f.alternate() {
            crate::print::print(self)
        } else {
            crate::print::print_unformatted(self)
        };
        match result {
            Ok(s) => f.write_str(&s),
            Err(_) => Err(std::fmt::Error),
        }
    }
}

impl std::str::FromStr for Value {
    type Err = crate::error::CJsonError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        crate::parse::parse(s)
    }
}

#[cfg(test)]
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;

    #[test]
    fn constructors_and_predicates() {
        assert!(Value::null().is_null());
        assert!(Value::boolean(true).is_true());
        assert!(Value::boolean(false).is_false());
        assert!(Value::number(3.5).is_number());
        assert!(Value::string("hi").is_string());
        assert!(Value::array().is_array());
        assert!(Value::object().is_object());
    }

    #[test]
    fn array_push_and_get() {
        let mut arr = Value::array();
        arr.array_push(Value::number(1.0)).unwrap();
        arr.array_push(Value::number(2.0)).unwrap();
        assert_eq!(arr.array_len(), Some(2));
        assert_eq!(arr.array_get(1).unwrap().as_f64(), Some(2.0));
    }

    #[test]
    fn array_push_wrong_type_returns_item_back() {
        let mut not_arr = Value::null();
        let err = not_arr.array_push(Value::number(1.0)).unwrap_err();
        assert_eq!(err, Value::number(1.0));
    }

    #[test]
    fn object_push_and_lookup() {
        let mut obj = Value::object();
        obj.object_push("Name", Value::string("cJSON")).unwrap();
        assert_eq!(obj.object_get("Name").unwrap().as_str(), Some("cJSON"));
        assert_eq!(obj.object_get("name"), None); // case-sensitive
        assert_eq!(obj.object_get_ci("name").unwrap().as_str(), Some("cJSON"));
        assert!(obj.object_has("Name"));
        assert!(!obj.object_has("Missing"));
    }

    #[test]
    fn duplicate_keys_first_match_wins() {
        let mut obj = Value::object();
        obj.object_push("k", Value::number(1.0)).unwrap();
        obj.object_push("k", Value::number(2.0)).unwrap();
        assert_eq!(obj.object_get("k").unwrap().as_f64(), Some(1.0));
    }

    #[test]
    fn as_int_truncates_like_c_cast() {
        assert_eq!(Value::number(3.9).as_int(), Some(3));
        assert_eq!(Value::number(-3.9).as_int(), Some(-3));
    }

    // --- Phase 5: detach / delete / insert / replace ---

    #[test]
    fn array_detach_removes_and_returns_shifting_rest_left() {
        let mut arr = Value::Array(vec![
            Value::number(1.0),
            Value::number(2.0),
            Value::number(3.0),
        ]);
        let detached = arr.array_detach(1).unwrap();
        assert_eq!(detached, Value::number(2.0));
        assert_eq!(
            arr,
            Value::Array(vec![Value::number(1.0), Value::number(3.0)])
        );
        assert_eq!(arr.array_detach(99), None);
    }

    #[test]
    fn object_detach_case_sensitivity() {
        let mut obj = Value::Object(vec![("Name".to_string(), Value::string("cJSON"))]);
        assert_eq!(obj.clone().object_detach("name", true), None);
        assert_eq!(
            obj.object_detach("name", false),
            Some(Value::string("cJSON"))
        );
        assert!(obj.as_object().unwrap().is_empty());
    }

    #[test]
    fn array_insert_shifts_right_and_appends_past_end() {
        let mut arr = Value::Array(vec![Value::number(1.0), Value::number(3.0)]);
        arr.array_insert(1, Value::number(2.0)).unwrap();
        assert_eq!(
            arr,
            Value::Array(vec![
                Value::number(1.0),
                Value::number(2.0),
                Value::number(3.0)
            ])
        );
        arr.array_insert(99, Value::number(4.0)).unwrap();
        assert_eq!(arr.array_get(3), Some(&Value::number(4.0)));
    }

    #[test]
    fn array_replace_in_place_and_out_of_bounds_fails() {
        let mut arr = Value::Array(vec![Value::number(1.0), Value::number(2.0)]);
        assert!(arr.array_replace(0, Value::string("x")));
        assert_eq!(arr.array_get(0), Some(&Value::string("x")));
        assert!(!arr.array_replace(5, Value::number(9.0)));
    }

    #[test]
    fn object_replace_renames_key_to_search_string() {
        // Matches replace_item_in_object's real behavior: the matched key's
        // original casing is overwritten by the search key.
        let mut obj = Value::Object(vec![("Name".to_string(), Value::number(1.0))]);
        assert!(obj.object_replace("name", false, Value::number(2.0)));
        assert_eq!(
            obj.as_object().unwrap(),
            &[("name".to_string(), Value::number(2.0))]
        );
    }

    #[test]
    fn object_replace_missing_key_fails() {
        let mut obj = Value::object();
        assert!(!obj.object_replace("missing", true, Value::number(1.0)));
    }

    // --- Phase 5: duplicate ---

    #[test]
    fn duplicate_recurse_true_deep_copies() {
        let v = Value::Array(vec![Value::number(1.0), Value::string("x")]);
        assert_eq!(v.duplicate(true), v);
    }

    #[test]
    fn duplicate_recurse_false_empties_containers() {
        let v = Value::Array(vec![Value::number(1.0), Value::string("x")]);
        assert_eq!(v.duplicate(false), Value::array());

        let o = Value::Object(vec![("a".to_string(), Value::number(1.0))]);
        assert_eq!(o.duplicate(false), Value::object());

        // Scalars are unaffected by recurse.
        assert_eq!(Value::number(3.5).duplicate(false), Value::number(3.5));
    }

    // --- Phase 5: compare ---

    #[test]
    fn compare_numbers_uses_epsilon_not_bit_equality() {
        assert!(compare(
            &Value::number(0.1 + 0.2),
            &Value::number(0.3),
            true
        ));
    }

    #[test]
    fn compare_arrays_are_order_sensitive() {
        let a = Value::Array(vec![Value::number(1.0), Value::number(2.0)]);
        let b = Value::Array(vec![Value::number(2.0), Value::number(1.0)]);
        assert!(!compare(&a, &b, true));
    }

    #[test]
    fn compare_objects_are_order_insensitive_but_key_set_must_match() {
        let a = Value::Object(vec![
            ("a".to_string(), Value::number(1.0)),
            ("b".to_string(), Value::number(2.0)),
        ]);
        let b = Value::Object(vec![
            ("b".to_string(), Value::number(2.0)),
            ("a".to_string(), Value::number(1.0)),
        ]);
        assert!(compare(&a, &b, true));

        let c = Value::Object(vec![("a".to_string(), Value::number(1.0))]);
        assert!(!compare(&a, &c, true)); // b has an extra key
        assert!(!compare(&c, &a, true)); // a is missing from c
    }

    #[test]
    fn compare_object_case_sensitivity() {
        let a = Value::Object(vec![("Name".to_string(), Value::number(1.0))]);
        let b = Value::Object(vec![("name".to_string(), Value::number(1.0))]);
        assert!(!compare(&a, &b, true));
        assert!(compare(&a, &b, false));
    }

    #[test]
    fn compare_different_types_are_never_equal() {
        assert!(!compare(&Value::null(), &Value::boolean(false), true));
        assert!(!compare(&Value::number(0.0), &Value::string(""), true));
    }

    // --- Phase 5: AddXToObject convenience constructors ---

    #[test]
    fn add_x_to_object_helpers() {
        let mut obj = Value::object();
        assert!(obj.add_null_to_object("n"));
        assert!(obj.add_bool_to_object("b", true));
        assert!(obj.add_number_to_object("num", 3.5));
        assert!(obj.add_string_to_object("s", "hi"));
        assert!(obj.add_array_to_object("arr"));
        assert!(obj.add_object_to_object("obj"));

        assert_eq!(obj.object_get("n"), Some(&Value::null()));
        assert_eq!(obj.object_get("b"), Some(&Value::boolean(true)));
        assert_eq!(obj.object_get("num"), Some(&Value::number(3.5)));
        assert_eq!(obj.object_get("s"), Some(&Value::string("hi")));
        assert_eq!(obj.object_get("arr"), Some(&Value::array()));
        assert_eq!(obj.object_get("obj"), Some(&Value::object()));

        // Fails (returns false) when `self` isn't an Object.
        let mut not_obj = Value::null();
        assert!(!not_obj.add_bool_to_object("x", true));
    }

    // --- Display and FromStr ---

    #[test]
    fn display_compact_output() {
        let v = Value::Object(vec![
            ("a".to_string(), Value::number(1.0)),
            ("b".to_string(), Value::Array(vec![Value::Bool(true), Value::Null])),
        ]);
        assert_eq!(format!("{}", v), r#"{"a":1,"b":[true,null]}"#);
    }

    #[test]
    fn display_pretty_output_with_alternate() {
        let v = Value::Object(vec![
            ("a".to_string(), Value::number(1.0)),
        ]);
        let pretty = format!("{:#}", v);
        assert!(pretty.contains('\n'));
        assert!(pretty.contains('\t'));
        assert!(pretty.contains("\"a\""));
    }

    #[test]
    fn from_str_parses_valid_json() {
        let v: Value = r#"{"key": [1, 2, 3]}"#.parse().unwrap();
        assert!(v.is_object());
        assert_eq!(v.object_get("key").unwrap().array_len(), Some(3));
    }

    #[test]
    fn from_str_rejects_invalid_json() {
        let result: Result<Value, _> = "{invalid".parse();
        assert!(result.is_err());
    }

    #[test]
    fn display_roundtrips_through_from_str() {
        let original = Value::Array(vec![
            Value::number(1.0),
            Value::string("hello"),
            Value::Bool(false),
            Value::Null,
        ]);
        let serialized = format!("{}", original);
        let parsed: Value = serialized.parse().unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn error_display_is_human_readable() {
        let err = crate::error::CJsonError::UnexpectedToken { pos: 42 };
        assert_eq!(format!("{}", err), "unexpected token at position 42");
    }
}
