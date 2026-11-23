include "controls";

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
def target_lock($r):
  if $r.build.target_dir.lock == "cooperating-measure-script-only" then
    "cooperative"
  elif $r.build.target_dir.lock == "not-recorded" then
    "not-recorded"
  else
    "unknown"
  end;
# A strict comparison needs every control to be observed. The definition is
# shared with the conflict validator in controls.jq.
def controls_complete($r): $r | controls_complete;
def controls_text($r):
  if controls_complete($r) then "complete" else "incomplete" end;
# Reasons derived from the controls that define series_id. Offline and locked
# are build controls and belong here; source state is reported separately.
def control_diff($p; $r):
  [
    (if $p.toolchain != $r.toolchain then "toolchain" else empty end),
    (if $p.platform.host != $r.platform.host then "host" else empty end),
    (if $p.build.requested_features != $r.build.requested_features then
       "requested-features" else empty end),
    (if $p.build.requested_no_default_features != $r.build.requested_no_default_features then
       "requested-no-default" else empty end),
    (if $p.build.profile != $r.build.profile then "profile" else empty end),
    (if $p.build.offline != $r.build.offline then "offline" else empty end),
    (if $p.build.locked != $r.build.locked then "locked" else empty end),
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
    (if $p.build.effective_features == $r.build.effective_features then empty
     elif ($p.build.effective_features.status == "not-captured" or
           $r.build.effective_features.status == "not-captured") then
       "effective-features-capture"
     else "effective-features" end),
    (if $p.build.effective_linker == $r.build.effective_linker then empty
     elif ($p.build.effective_linker.status == "not-captured" or
           $r.build.effective_linker.status == "not-captured") then
       "effective-linker-capture"
     else "effective-linker" end),
    (if $p.build.v8_archive_status != $r.build.v8_archive_status or
        v8_identity($p) != v8_identity($r) then
       "v8-archive" else empty end)
  ] | join(",");
def source_diff($p; $r):
  [
    (if $p.source.git_commit != $r.source.git_commit then "commit" else empty end),
    (if $p.source.git_canonical != $r.source.git_canonical then
       "canonical" else empty end),
    (if ($p.source.state_sha256 // $p.source.state_status) ==
        ($r.source.state_sha256 // $r.source.state_status) then empty
     elif ($p.source.state_status == "not-captured" or
           $r.source.state_status == "not-captured") then
       "state-capture"
     else "state" end)
  ] | join(",");
def same_source_state($p; $r):
  ($p.source.git_commit == $r.source.git_commit) and
  (($p.source.state_sha256 // $p.source.state_status) ==
   ($r.source.state_sha256 // $r.source.state_status));
def conflict_groups($all):
  [$all[] | select(.status == "ok")]
  | group_by(conflict_key)
  | map(select((map(.measurement.sha256) | unique | length) > 1))
  | map({
      kind: conflict_group_kind,
      records: .
    });
def conflict_status($groups; $r):
  ($groups
   | map(select(.records[0] | conflict_key == conflict_key($r)))
   | .[0].kind // "no");

. as $all
| (conflict_groups($all)) as $conflicts
| if $mode == "trend" then
    (["family", "package", "target", "measured_at", "commit", "canonical",
      "target-lock", "controls", "bytes", "delta", "delta-kind",
      "controls-changed", "source-changed", "series", "conflict"] | @tsv),
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
              $r.measurement.sha256 != $p.measurement.sha256) then
          (if (controls_complete($p) and controls_complete($r))
           then "conflict" else "possible-conflict" end)
        elif ($r.series_id == $p.series_id and
              ((controls_complete($p) and controls_complete($r)) | not)) then
          "controls-incomplete"
        elif ($r.series_id == $p.series_id and
              ($r.source.git_canonical != $p.source.git_canonical or
               $r.source.state_sha256 != $p.source.state_sha256)) then
          "source-state"
        elif $r.series_id == $p.series_id then "strict"
        else "controls-differ" end) as $kind
     | (if $p == null then "-"
        else (control_diff($p; $r) | if . == "" then "-" else . end) end) as $changed
     | (if $p == null then "-"
        else (source_diff($p; $r) | if . == "" then "-" else . end) end) as $source_changed
     | [
         ($r.family_id | short),
         $r.artifact.package,
         $r.platform.target,
         $r.measured_at,
         ($r.source.git_commit | short),
         (if $r.source.git_canonical then "yes" else "no" end),
         target_lock($r),
         controls_text($r),
         bytes_text($r),
         (if $delta == null then "-"
          elif $delta > 0 then "+\($delta)"
          else ($delta | tostring) end),
         $kind,
         $changed,
         $source_changed,
         ($r.series_id | short),
         (conflict_status($conflicts; $r))
       ] | @tsv)
  elif $mode == "all" then
    (["measured_at", "status", "source", "family", "series", "package",
      "target", "rustc", "requested-features", "target-lock", "controls",
      "bytes", "sha256", "commit", "canonical", "v8"] | @tsv),
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
         target_lock(.),
         controls_text(.),
         bytes_text(.),
         digest_text(.),
         (.source.git_commit | short),
         (if .source.git_canonical then "yes" else "no" end),
         .build.v8_archive_status
       ] | @tsv)
  else
    (["series", "family", "package", "target", "measured_at", "status",
      "source", "target-lock", "controls", "rustc", "requested-features",
      "bytes", "sha256", "commit", "canonical", "v8", "conflict"] | @tsv),
    (sort_by(.measured_at)
     | group_by(.series_id)
     | map(last)
     | sort_by([.artifact.package, .platform.target, .toolchain.rustc, .series_id])[]
     | . as $r
     | [
         ($r.series_id | short),
         ($r.family_id | short),
         $r.artifact.package,
         $r.platform.target,
         $r.measured_at,
         $r.status,
         $r.source.kind,
         target_lock($r),
         controls_text($r),
         $r.toolchain.rustc,
         feature_text($r),
         bytes_text($r),
         digest_text($r),
         ($r.source.git_commit | short),
         (if $r.source.git_canonical then "yes" else "no" end),
         $r.build.v8_archive_status,
         (conflict_status($conflicts; $r))
       ] | @tsv)
  end
