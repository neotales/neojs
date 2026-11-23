def short: .[0:12];
def feature_text($r):
  if ($r.build.requested_features | length) == 0 then "-"
  else ($r.build.requested_features | join(","))
  end;
def bytes_text($r):
  if $r.measurement.bytes == null then "-"
  else ($r.measurement.bytes | tostring)
  end;
def digest_text($r):
  if $r.measurement.sha256 == null then "-"
  else ($r.measurement.sha256 | short)
  end;
def v8_identity($r):
  if $r.build.v8_archive == null then null
  else {
    name: $r.build.v8_archive.name,
    sha256: $r.build.v8_archive.sha256
  }
  end;
def control_diff($p; $r):
  [
    (if $p.toolchain != $r.toolchain then "toolchain" else empty end),
    (if $p.platform.host != $r.platform.host then "host" else empty end),
    (if $p.build.requested_features != $r.build.requested_features then
       "requested-features" else empty end),
    (if $p.build.requested_no_default_features != $r.build.requested_no_default_features then
       "requested-no-default" else empty end),
    (if $p.build.profile != $r.build.profile then "profile" else empty end),
    (if $p.build.cargo_lock_sha256 != $r.build.cargo_lock_sha256 then
       "cargo-lock" else empty end),
    (if $p.build.workspace_path_sha256 != $r.build.workspace_path_sha256 then
       "workspace-path" else empty end),
    (if $p.build.cargo_config == $r.build.cargo_config then empty
     elif ($p.build.cargo_config.status == "not-captured" or
           $r.build.cargo_config.status == "not-captured") then
       "cargo-config-capture"
     else "cargo-config" end),
    (if $p.build.env_overrides != $r.build.env_overrides then
       "env-overrides" else empty end),
    (if $p.build.target_specific_env == $r.build.target_specific_env then empty
     elif ($p.build.target_specific_env.status == "not-captured" or
           $r.build.target_specific_env.status == "not-captured") then
       "target-env-capture"
     else "target-env" end),
    (if $p.build.effective_features != $r.build.effective_features then
       "effective-features" else empty end),
    (if $p.build.effective_linker != $r.build.effective_linker then
       "effective-linker" else empty end),
    (if $p.build.v8_archive_status != $r.build.v8_archive_status or
        v8_identity($p) != v8_identity($r) then
       "v8-archive" else empty end),
    (if $p.source.git_canonical != $r.source.git_canonical then
       "source-canonical"
     elif $p.source.state_sha256 == $r.source.state_sha256 then empty
     elif ($p.source.state_status == "not-captured" or
           $r.source.state_status == "not-captured") then
       "source-state-capture"
     else "source-state" end)
  ] | join(",");
def same_source_state($p; $r):
  ($p.source.git_commit == $r.source.git_commit) and
  (($p.source.state_sha256 // $p.source.state_status) ==
   ($r.source.state_sha256 // $r.source.state_status));
def conflict_series($all):
  [$all[] | select(.status == "ok")]
  | group_by([
      .series_id,
      .source.git_commit,
      .source.git_dirty,
      (.source.state_sha256 // .source.state_status)
    ])
  | map(select((map(.measurement.sha256) | unique | length) > 1))
  | map(.[0].series_id)
  | unique;

. as $all
| (conflict_series($all)) as $conflicts
| if $mode == "trend" then
    (["family", "measured_at", "commit", "canonical", "bytes", "delta",
      "delta-kind", "controls-changed", "series", "conflict"] | @tsv),
    (sort_by([.family_id, .measured_at])
     | group_by(.family_id)[]
     | sort_by(.measured_at) as $g
     | range(0; $g | length) as $i
     | $g[$i] as $r
     | (if $i == 0 then null else $g[$i - 1] end) as $p
     | ($r.measurement.bytes) as $rb
     | (if $p == null then null else $p.measurement.bytes end) as $pb
     | (if $p == null or $rb == null or $pb == null then null
        else $rb - $pb end) as $delta
     | (if $p == null then "baseline"
        elif $r.status == "missing" then "missing"
        elif $p.status == "missing" then "resumed"
        elif ($r.series_id == $p.series_id and
              same_source_state($p; $r) and
              $r.measurement.sha256 != $p.measurement.sha256) then "conflict"
        elif ($r.series_id == $p.series_id and
              $r.source.git_canonical == $p.source.git_canonical and
              $r.source.state_sha256 == $p.source.state_sha256) then "strict"
        elif $r.series_id == $p.series_id then "source-state"
        else "controls-differ" end) as $kind
     | (if $p == null then "-"
        else (control_diff($p; $r) | if . == "" then "-" else . end) end) as $changed
     | [
         ($r.family_id | short),
         $r.measured_at,
         ($r.source.git_commit | short),
         (if $r.source.git_canonical then "yes" else "no" end),
         bytes_text($r),
         (if $delta == null then "-"
          elif $delta > 0 then "+\($delta)"
          else ($delta | tostring) end),
         $kind,
         $changed,
         ($r.series_id | short),
         (if ($conflicts | index($r.series_id)) then "yes" else "no" end)
       ] | @tsv)
  elif $mode == "all" then
    (["measured_at", "status", "source", "family", "series", "package",
      "target", "rustc", "requested-features", "bytes", "sha256", "commit",
      "canonical", "v8"] | @tsv),
    (sort_by(.measured_at)[]
     | [
         .measured_at,
         .status,
         .source.kind,
         (.family_id | short),
         (.series_id | short),
         .artifact.package,
         .platform.target,
         .toolchain.rustc,
         feature_text(.),
         bytes_text(.),
         digest_text(.),
         (.source.git_commit | short),
         (if .source.git_canonical then "yes" else "no" end),
         .build.v8_archive_status
       ] | @tsv)
  else
    (["series", "family", "measured_at", "status", "source", "package",
      "target", "rustc", "requested-features", "bytes", "sha256", "commit",
      "canonical", "v8", "conflict"] | @tsv),
    (sort_by(.measured_at)
     | group_by(.series_id)
     | map(last)
     | sort_by([.artifact.package, .platform.target, .toolchain.rustc, .series_id])[]
     | . as $r
     | [
         ($r.series_id | short),
         ($r.family_id | short),
         $r.measured_at,
         $r.status,
         $r.source.kind,
         $r.artifact.package,
         $r.platform.target,
         $r.toolchain.rustc,
         feature_text($r),
         bytes_text($r),
         digest_text($r),
         ($r.source.git_commit | short),
         (if $r.source.git_canonical then "yes" else "no" end),
         $r.build.v8_archive_status,
         (if ($conflicts | index($r.series_id)) then "yes" else "no" end)
       ] | @tsv)
  end
