# Sourced by publish.yml and rollback.yml before any aws call: the R2_* secrets → the aws CLI's
# AWS_* settings. Trims stray whitespace (a pasted secret often ends with a line break), checks
# the endpoint's shape (release-lib.mjs r2Endpoint) and masks the cleaned values. Prints no value.
r2_trim() { printf '%s' "$1" | tr -d '[:space:]'; }
r2_id=$(r2_trim "${R2_ACCESS_KEY_ID:-}")
r2_key=$(r2_trim "${R2_SECRET_ACCESS_KEY:-}")
if [ -z "$r2_id" ] || [ -z "$r2_key" ] || [ -z "${R2_ENDPOINT:-}" ]; then
  echo "::error::R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY and R2_ENDPOINT are required (docs/RELEASING.md)"
  exit 1
fi
if ! r2_endpoint=$(node .github/scripts/release.mjs r2-endpoint); then
  echo "::error::R2_ENDPOINT is not usable (the reason is on the line above; the value is never shown)"
  exit 1
fi
echo "::add-mask::$r2_id"
echo "::add-mask::$r2_key"
echo "::add-mask::$r2_endpoint"
export AWS_ACCESS_KEY_ID="$r2_id" AWS_SECRET_ACCESS_KEY="$r2_key" AWS_ENDPOINT_URL="$r2_endpoint"
export AWS_DEFAULT_REGION=auto AWS_REQUEST_CHECKSUM_CALCULATION=when_required AWS_RESPONSE_CHECKSUM_VALIDATION=when_required
unset r2_id r2_key r2_endpoint
