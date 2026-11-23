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
