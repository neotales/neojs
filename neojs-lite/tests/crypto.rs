//! End-to-end tests for the `neo:crypto` built-in through the `neojs-lite`
//! command, which enables the `crypto` feature explicitly.

use std::fs;
use std::process::Command;

fn run_module(source: &str) -> std::process::Output {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    Command::new(env!("CARGO_BIN_EXE_neojs-lite"))
        .arg(file)
        .output()
        .expect("run neojs-lite")
}

#[test]
fn the_command_can_import_and_use_neo_crypto() {
    let output = run_module(
        r#"
import { decrypt, encrypt, generateEnvelopeKey, randomBytes } from "crypto";
import * as scheme from "neo:crypto";

if (scheme.encrypt !== encrypt) throw new Error("module identities differ");
const key = generateEnvelopeKey();
const plaintext = Uint8Array.from([1, 2, 3, 255]);
const out = decrypt(key, encrypt(key, plaintext));
if (out.length !== plaintext.length) throw new Error("roundtrip length");
for (let i = 0; i < out.length; i++) {
    if (out[i] !== plaintext[i]) throw new Error(`roundtrip byte ${i}`);
}
if (randomBytes(4).length !== 4) throw new Error("randomBytes");
console.log("crypto ok");
"#,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "crypto ok\n");
}

#[test]
fn an_unimplemented_export_is_a_link_error() {
    let output = run_module("import { hashAsync } from \"neo:crypto\";\n");
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("Could not find export")
            || error.contains("does not provide an export named"),
        "{error}"
    );
}
