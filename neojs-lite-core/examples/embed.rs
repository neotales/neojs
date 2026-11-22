//! Embedding example for `neojs-lite-core`.
//!
//! This is the primary way to use NeoJS Lite: construct a [`Runtime`] in your
//! process and run an ECMAScript module file. The `neojs-lite` executable is
//! only a secondary convenience wrapper around this API.
//!
//! ```sh
//! cargo run -p neojs-lite-core --example embed -- examples/hello.js
//! ```

use std::process::ExitCode;

use neojs_lite_core::Runtime;

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: embed <module.js>");
        return ExitCode::from(2);
    };

    let mut runtime = match Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("embed: {error}");
            return ExitCode::from(2);
        }
    };

    match runtime.run_file(path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("embed: {error}");
            ExitCode::from(1)
        }
    }
}
