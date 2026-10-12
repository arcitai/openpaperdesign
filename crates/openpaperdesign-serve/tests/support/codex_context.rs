//! Synthetic envelopes shaped from the operator's Codex 0.162.1 model-call receipt.
//! These test the adapter; they are not evidence of installed Codex execution.
use rmcp::model::RequestMetaObject;
use serde_json::{Value, json};
use std::path::Path;

pub const KEY: &str = "codex/sandbox-state-meta";

pub fn uri(path: &Path) -> String {
    let mut s = String::from("file://");
    for b in path.to_str().unwrap().bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            s.push(b as char);
        } else {
            use std::fmt::Write;
            write!(s, "%{b:02X}").unwrap();
        }
    }
    s
}

pub fn writable(path: &Path) -> RequestMetaObject {
    let mut state: Value =
        serde_json::from_str(include_str!("../fixtures/codex-sandbox-state.json")).unwrap();
    fn substitute(value: &mut Value, path: &str) {
        match value {
            Value::String(s) => *s = s.replace("{workspace}", path),
            Value::Array(a) => a.iter_mut().for_each(|v| substitute(v, path)),
            Value::Object(o) => o.values_mut().for_each(|v| substitute(v, path)),
            _ => {}
        }
    }
    substitute(&mut state, path.to_str().unwrap());
    state["sandboxCwd"] = uri(path).into();
    json!({KEY:state}).as_object().unwrap().clone().into()
}

pub fn readonly(path: &Path) -> RequestMetaObject {
    let mut meta = writable(path);
    let entries = &mut meta.get_mut(KEY).unwrap()["permissionProfile"]["file_system"]["entries"];
    *entries = json!([entries[0].clone()]);
    meta
}

pub fn disabled(path: &Path) -> RequestMetaObject {
    let mut meta = writable(path);
    meta.get_mut(KEY).unwrap()["permissionProfile"] = json!({"type":"disabled"});
    meta
}
