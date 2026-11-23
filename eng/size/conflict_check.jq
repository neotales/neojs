include "controls";

# Emit one line per differing-digest group:
#   confirmed<TAB>package<TAB>target<TAB>series<TAB>commit<TAB>digests
#   possible<TAB>...
# A group is confirmed only when every record in it has complete controls.
map(select(.status == "ok"))
| group_by([
    .series_id,
    .source.git_commit,
    .source.git_dirty,
    (.source.state_sha256 // .source.state_status)
  ])
| map(select((map(.measurement.sha256) | unique | length) > 1))
| .[]
| . as $g
| (if ($g | map(controls_complete) | index(false) == null)
   then "confirmed" else "possible" end) as $kind
| [
    $kind,
    $g[0].artifact.package,
    $g[0].platform.target,
    ($g[0].series_id[0:12]),
    ($g[0].source.git_commit[0:12]),
    ($g | map(.measurement.sha256[0:12]) | join(","))
  ] | @tsv
