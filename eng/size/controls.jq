# Whether a record has every comparison control captured. A differing digest
# with incomplete controls is only a possible conflict: an uncaptured effective
# feature/linker, config, target environment, source state, target lock or V8
# input may explain it.
def controls_complete:
  (.build.effective_features.status == "captured") and
  (.build.effective_linker.status == "captured") and
  (.build.cargo_config.status == "captured") and
  (.build.target_specific_env.status != "not-captured") and
  (.build.target_dir.lock == "cooperating-measure-script-only") and
  (.source.state_status == "captured") and
  (.build.v8_archive_status != "supplied-provenance-unknown") and
  (.build.v8_archive_status != "not-supplied-not-captured");

# The group key shared by conflict validation and reporting. Two records are in
# the same reproducibility group only when series, commit, dirty flag and source
# state match.
def conflict_key:
  [
    .series_id,
    .source.git_commit,
    .source.git_dirty,
    (.source.state_sha256 // .source.state_status)
  ];
def conflict_key($r): $r | conflict_key;

# Mixed completeness is possible because the target-lock contract is not part
# of series_id. A group is confirmed only when every record has complete
# controls; one incomplete record makes the whole group possible.
def conflict_group_complete:
  map(controls_complete) | index(false) == null;
def conflict_group_kind:
  if conflict_group_complete then "confirmed" else "possible" end;
