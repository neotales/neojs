def asstr: (. // "") | tostring;
def slug: test("^[a-z][a-z0-9_-]*$");
def scope: test("^[a-z][a-z0-9-]*$");
def triple: test("^[A-Za-z0-9_.-]+$");
def hex64: test("^[0-9a-f]{64}$");
def commit: test("^([0-9a-f]{40}|[0-9a-f]{64})$");
def iso: test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$");
def reason: test("^[a-z0-9][a-z0-9-]*$");
def isint: (type == "number") and (. == floor);
def isstrarr: (type == "array") and all(.[]; type == "string");
def status_in($allowed): .status as $s | ($allowed | index($s)) != null;

try (
[
  (if type != "object" then "record must be a JSON object" else empty end),
  (if .schema != 3 then "schema must be 3" else empty end),
  (if .status != "ok" and .status != "missing" then
     "status must be \"ok\" or \"missing\""
   else empty end),
  (if has("context_key") then
     "legacy context_key is not part of schema 3"
   else empty end),

  # Artifact
  (if (.artifact | type) != "object" then
     "artifact must be an object"
   elif (.artifact.package | asstr | slug | not) then
     "artifact.package must be a lowercase package name"
   else empty end),
  (if (.artifact.bin | asstr | slug | not) then
     "artifact.bin must be a lowercase binary name"
   else empty end),
  (if .artifact.kind != "release-binary" then
     "artifact.kind must be \"release-binary\""
   else empty end),
  (if (.artifact.path | asstr | test("^[^/\\\\]+$") | not) then
     "artifact.path must be a basename without a directory"
   else empty end),

  # Source
  (if (.source | type) != "object" then
     "source must be an object"
   elif (.source.git_commit | asstr | commit | not) then
     "source.git_commit must be a 40 or 64 character lowercase hex id"
   else empty end),
  (if (.source.git_dirty | type) != "boolean" then
     "source.git_dirty must be a boolean"
   else empty end),
  (if (.source.git_canonical | type) != "boolean" then
     "source.git_canonical must be a boolean"
   elif .source.git_canonical != (.source.git_dirty | not) then
     "source.git_canonical must be the inverse of source.git_dirty"
   else empty end),
  (if .source.state_status != "captured" and .source.state_status != "not-captured" then
     "source.state_status must be \"captured\" or \"not-captured\""
   elif .source.state_status == "captured" and
        (.source.state_sha256 | asstr | hex64 | not) then
     "source.state_sha256 must be a sha256 when state_status is captured"
   elif .source.state_status == "not-captured" and .source.state_sha256 != null then
     "source.state_sha256 must be null when state_status is not-captured"
   else empty end),
  (if (.source.git_branch | asstr | length) == 0 then
     "source.git_branch must not be empty"
   else empty end),
  (if .source.kind != "tag" and .source.kind != "commit" then
     "source.kind must be \"tag\" or \"commit\""
   else empty end),
  (if .source.measured_from != "worktree" then
     "source.measured_from must be \"worktree\""
   else empty end),
  (if (.source.git_tags | isstrarr | not) then
     "source.git_tags must be an array of strings"
   else empty end),

  # Platform
  (if (.platform | type) != "object" then
     "platform must be an object"
   elif (.platform.target | asstr | triple | not) then
     "platform.target must be a target triple"
   else empty end),
  (if (.platform.host | asstr | triple | not) then
     "platform.host must be a target triple"
   else empty end),
  (if (.platform.os | asstr | length) == 0 then
     "platform.os must not be empty"
   else empty end),
  (if (.platform.arch | asstr | length) == 0 then
     "platform.arch must not be empty"
   else empty end),

  # Toolchain
  (if (.toolchain | type) != "object" then
     "toolchain must be an object"
   elif (.toolchain.requested | asstr | length) == 0 then
     "toolchain.requested must not be empty"
   else empty end),
  (if (.toolchain.rustc | asstr | startswith("rustc ") | not) then
     "toolchain.rustc must start with \"rustc \""
   else empty end),
  (if (.toolchain.cargo | asstr | startswith("cargo ") | not) then
     "toolchain.cargo must start with \"cargo \""
   else empty end),

  # Build: only requested features are captured; effective features and the
  # effective linker are explicitly labelled unknown until a future version
  # records them.
  (if (.build | type) != "object" then
     "build must be an object"
   elif (.build | has("features") or has("no_default_features")) then
     "schema 3 uses requested_features and requested_no_default_features"
   elif .build.profile != "release" then
     "build.profile must be \"release\""
   else empty end),
  (if (.build.requested_features | isstrarr | not) then
     "build.requested_features must be an array of strings"
   else empty end),
  (if (.build.requested_no_default_features | type) != "boolean" then
     "build.requested_no_default_features must be a boolean"
   else empty end),
  (if (.build.effective_features | type) != "object" then
     "build.effective_features must be an object"
   elif (.build.effective_features | status_in(["not-captured", "captured"]) | not) then
     "build.effective_features.status must be not-captured or captured"
   elif .build.effective_features.status == "captured" and
        (.build.effective_features.values | isstrarr | not) then
     "build.effective_features.values must be strings when captured"
   else empty end),
  (if (.build.effective_linker | type) != "object" then
     "build.effective_linker must be an object"
   elif (.build.effective_linker | status_in(["not-captured", "captured"]) | not) then
     "build.effective_linker.status must be not-captured or captured"
   elif .build.effective_linker.status == "captured" and
        (.build.effective_linker.value | type) != "string" then
     "build.effective_linker.value must be a string when captured"
   else empty end),
  (if (.build.offline | type) != "boolean" then
     "build.offline must be a boolean"
   else empty end),
  (if (.build.locked | type) != "boolean" then
     "build.locked must be a boolean"
   else empty end),
  (if (.build.cargo_lock_sha256 | asstr | hex64 | not) then
     "build.cargo_lock_sha256 must be a sha256"
   else empty end),
  (if (.build.workspace_path_sha256 | asstr | hex64 | not) then
     "build.workspace_path_sha256 must be a sha256"
   else empty end),
  (if (.build.cargo_config | type) != "object" then
     "build.cargo_config must be an object"
   elif (.build.cargo_config | status_in(["captured", "not-captured"]) | not) then
     "build.cargo_config.status must be captured or not-captured"
   elif (.build.cargo_config.files | type) != "array" then
     "build.cargo_config.files must be an array"
   elif any(.build.cargo_config.files[];
            ((.scope | asstr | scope | not) or (.sha256 | asstr | hex64 | not))) then
     "build.cargo_config.files entries need a scope and sha256"
   elif .build.cargo_config.status == "not-captured" and
        (.build.cargo_config.files | length) != 0 then
     "build.cargo_config.files must be empty when not-captured"
   else empty end),
  (if (.build.env_overrides | type) != "object" then
     "build.env_overrides must be an object"
   elif any(.build.env_overrides[]; type != "string") then
     "build.env_overrides must contain only string values"
   else empty end),
  (if (.build.target_specific_env | type) != "object" then
     "build.target_specific_env must be an object"
   elif (.build.target_specific_env | status_in(["absent", "captured", "not-captured"]) | not) then
     "build.target_specific_env.status must be absent, captured or not-captured"
   elif (.build.target_specific_env.names | isstrarr | not) then
     "build.target_specific_env.names must be an array of strings"
   elif .build.target_specific_env.status != "captured" and
        (.build.target_specific_env.names | length) != 0 then
     "build.target_specific_env.names must be empty unless captured"
   else empty end),
  (if (.build.target_dir | type) != "object" then
     "build.target_dir must be an object"
   elif .build.target_dir.lock != "cooperating-measure-script-only" and
        .build.target_dir.lock != "not-recorded" then
     "build.target_dir.lock must be cooperating-measure-script-only or not-recorded"
   elif .build.target_dir.ownership != "exclusive-required" and
        .build.target_dir.ownership != "not-claimed" then
     "build.target_dir.ownership must be exclusive-required or not-claimed"
   elif .build.target_dir.external_writer_safety != "not-claimed" then
     "build.target_dir.external_writer_safety must remain not-claimed"
   elif .build.target_dir.lock == "not-recorded" and
        .build.target_dir.ownership != "not-claimed" then
     "an unrecorded target lock cannot claim exclusive ownership"
   elif .build.target_dir.lock == "cooperating-measure-script-only" and
        .build.target_dir.ownership != "exclusive-required" then
     "a cooperating target lock requires exclusive target ownership"
   else empty end),
  (if (.build.v8_archive_status | asstr |
       test("^(supplied-provenance-unknown|not-supplied-not-captured|not-applicable)$") | not) then
     "build.v8_archive_status is not a known value"
   elif .build.v8_archive_status == "supplied-provenance-unknown" then
     if (.build.v8_archive | type) != "object" then
       "build.v8_archive must be an object when supplied"
     elif (.build.v8_archive.name | asstr | test("^[^/\\\\]+$") | not) then
       "build.v8_archive.name must be a basename"
     elif (.build.v8_archive.sha256 | asstr | hex64 | not) then
       "build.v8_archive.sha256 must be a sha256"
     elif (.build.v8_archive.archive_sha256 | asstr | hex64 | not) then
       "build.v8_archive.archive_sha256 must be a sha256"
     elif (.build.v8_archive.provenance | asstr | length) == 0 then
       "build.v8_archive.provenance must not be empty"
     elif .build.v8_archive.authenticated != false then
       "build.v8_archive.authenticated must be false: a hash is not authentication"
     else empty end
   elif .build.v8_archive != null then
     "build.v8_archive must be null when no archive was supplied"
   else empty end),
  (if .status == "ok" then
     if (.build.command | asstr | length) == 0 then
       "build.command must not be empty for a measured artifact"
     elif (.build.command | contains("--release") | not) then
       "build.command must contain --release"
     elif (.build.command | contains("--locked") | not) then
       "build.command must contain --locked"
     elif (.build.command | contains("--target") | not) then
       "build.command must contain --target"
     elif (.build.command | test("--manifest-path|/home/|/Users/|/tmp/|/var/folders/")) then
       "build.command must not contain a machine-specific path"
     else empty end
   else empty end),

  # Measurement
  (if .status == "ok" then
     if (.measurement | type) != "object" then
       "measurement must be an object"
     elif .measurement.method != "file-length-bytes" then
       "measurement.method must be \"file-length-bytes\""
     elif (.measurement.bytes | isint | not) then
       "measurement.bytes must be an integer"
     elif (.measurement.bytes <= 0) then
       "measurement.bytes must be greater than zero"
     elif (.measurement.sha256 | asstr | hex64 | not) then
       "measurement.sha256 must be a sha256"
     elif (.measurement.file | type) != "null" and (.measurement.file | type) != "string" then
       "measurement.file must be null or a string"
     else empty end
   else
     if (.measurement | type) != "object" then
       "measurement must be an object"
     elif (.measurement.bytes != null or .measurement.sha256 != null or
           .measurement.file != null or .measurement.method != null) then
       "a missing artifact must not have a measurement"
     else empty end
   end),
  (if .status == "missing" then
     if (.missing_reason | asstr | reason | not) then
       "missing_reason must be a lowercase reason"
     else empty end
   elif .missing_reason != null then
     "missing_reason must be null for a measured artifact"
   else empty end),

  (if (.measured_at | asstr | iso | not) then
     "measured_at must be UTC ISO-8601 with second precision"
   else empty end),
  (if (.notes | type) != "null" and (.notes | type) != "string" then
     "notes must be null or a string"
   else empty end),
  (if (.key | asstr | hex64 | not) then
     "key must be a sha256"
   else empty end),
  (if (.series_id | asstr | hex64 | not) then
     "series_id must be a sha256"
   else empty end),
  (if (.family_id | asstr | hex64 | not) then
     "family_id must be a sha256"
   else empty end)
] | .[]
) catch "record shape is invalid: \(.)"
