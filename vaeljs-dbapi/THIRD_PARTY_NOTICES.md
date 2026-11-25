# Third-party notices

`vaeljs-dbapi` is MIT-licensed. It depends on published crates; no third-party
source is copied into this crate.

| Dependency | Version | License |
| --- | --- | --- |
| `neotales-dbapi` | 0.0.0-alpha.0 | MIT |
| `neotales-dbapi-sqlite` | 0.0.0-alpha.0 | MIT |
| `serde_json` | 1.0 | MIT OR Apache-2.0 |
| `tokio` (dev only) | 1 | MIT |
| `tempfile` (dev only) | 3 | MIT OR Apache-2.0 |

The SQLite provider's transitive graph is permissive: MIT, Apache-2.0,
Unicode-3.0, Unlicense OR MIT, and Zlib. `cargo tree -p vaeljs-dbapi --format
'{p} :: {l}'` is the check used for this list.
