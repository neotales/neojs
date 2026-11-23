include "controls";

# Emit one line per differing-digest group:
#   confirmed<TAB>package<TAB>target<TAB>series<TAB>commit<TAB>digests
#   possible<TAB>...
# The group kind is the same rule report.jq uses.
map(select(.status == "ok"))
| group_by(conflict_key)
| map(select((map(.measurement.sha256) | unique | length) > 1))
| .[]
| . as $g
| [
    ($g | conflict_group_kind),
    $g[0].artifact.package,
    $g[0].platform.target,
    ($g[0].series_id[0:12]),
    ($g[0].source.git_commit[0:12]),
    ($g | map(.measurement.sha256[0:12]) | join(","))
  ] | @tsv
