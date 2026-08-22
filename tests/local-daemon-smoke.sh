#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/gitmesh-smoke.XXXXXX")"
SOCKET_PATH="$TMP_DIR/gitmeshd.sock"
OBJECT_STORE="$TMP_DIR/objects.tsv"
REF_STORE="$TMP_DIR/refs.tsv"
POLICY_STORE="$TMP_DIR/policy.tsv"
KEY_STORE="$TMP_DIR/key-grants.tsv"
ACCOUNT_STORE="$TMP_DIR/accounts.tsv"
COLLAB_STORE="$TMP_DIR/collaboration.tsv"
DAEMON_PID=""
WEB_PID=""
WEB_PORT=""

cleanup() {
  if [[ -n "$WEB_PID" ]] && kill -0 "$WEB_PID" 2>/dev/null; then
    kill "$WEB_PID" 2>/dev/null || true
    wait "$WEB_PID" 2>/dev/null || true
  fi
  if [[ -n "$DAEMON_PID" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

run_gitmeshd() {
  cargo run --quiet --bin gitmeshd -- "$@"
}

expect_contains() {
  local haystack="$1"
  local needle="$2"
  if [[ "$haystack" != *"$needle"* ]]; then
    echo "expected response to contain '$needle'" >&2
    echo "response: $haystack" >&2
    exit 1
  fi
}

post_json() {
  local path="$1"
  local body="$2"
  curl -fsS \
    -H "content-type: application/json" \
    -d "$body" \
    "http://127.0.0.1:$WEB_PORT$path"
}

get_json() {
  local path="$1"
  curl -fsS "http://127.0.0.1:$WEB_PORT$path"
}

cd "$ROOT_DIR"

cargo run --quiet --bin gitmeshd -- serve \
  "$SOCKET_PATH" \
  "$OBJECT_STORE" \
  "$REF_STORE" \
  "$POLICY_STORE" \
  "$KEY_STORE" \
  "$ACCOUNT_STORE" \
  "$COLLAB_STORE" \
  >"$TMP_DIR/gitmeshd.log" 2>&1 &
DAEMON_PID="$!"

for _ in {1..80}; do
  if [[ -S "$SOCKET_PATH" ]] && run_gitmeshd ping "$SOCKET_PATH" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

ping_response="$(run_gitmeshd ping "$SOCKET_PATH")"
expect_contains "$ping_response" "OK pong"

issue_response="$(
  run_gitmeshd issue-open \
    "$SOCKET_PATH" \
    farzeen/gitmesh \
    farzeen \
    536d6f6b65206973737565 \
    7665726966696573206461656d6f6e20636f6c6c61626f726174696f6e20777269746573 \
    736d6f6b65,636f6c6c61626f726174696f6e
)"
expect_contains "$issue_response" "OK event="
expect_contains "$issue_response" "number=1"

pr_response="$(
  run_gitmeshd pr-open \
    "$SOCKET_PATH" \
    farzeen/gitmesh \
    farzeen \
    refs/heads/smoke \
    refs/heads/main \
    536d6f6b652070756c6c2072657175657374 \
    7665726966696573206461656d6f6e20707220777269746573 \
    736d6f6b65
)"
expect_contains "$pr_response" "OK event="
expect_contains "$pr_response" "number=1"

issues_response="$(run_gitmeshd issue-list "$SOCKET_PATH" farzeen/gitmesh)"
expect_contains "$issues_response" "count=1"
expect_contains "$issues_response" "536d6f6b65206973737565"

prs_response="$(run_gitmeshd pr-list "$SOCKET_PATH" farzeen/gitmesh)"
expect_contains "$prs_response" "count=1"
expect_contains "$prs_response" "refs/heads/smoke"

object_response="$(run_gitmeshd object-put "$SOCKET_PATH" blob 736d6f6b65206f626a656374)"
expect_contains "$object_response" "OK oid="
oid="$(sed -n 's/.*oid=\([0-9a-f]\{40\}\).*/\1/p' <<<"$object_response")"
if [[ -z "$oid" ]]; then
  echo "failed to parse object oid from: $object_response" >&2
  exit 1
fi

ref_response="$(run_gitmeshd ref-update "$SOCKET_PATH" tx-smoke refs/tags/smoke none "$oid" smoke)"
expect_contains "$ref_response" "status=committed"

status_response="$(run_gitmeshd repo-status "$SOCKET_PATH")"
expect_contains "$status_response" "objects=1"
expect_contains "$status_response" "refs=1"
expect_contains "$status_response" "collaboration_events=2"

npm --prefix apps/web run build >/dev/null
WEB_PORT="$(
  node -e "const net=require('net'); const s=net.createServer(); s.listen(0,'127.0.0.1',()=>{console.log(s.address().port); s.close();});"
)"
GITMESHD_SOCKET="$SOCKET_PATH" npm --prefix apps/web run start -- \
  -H 127.0.0.1 \
  -p "$WEB_PORT" \
  >"$TMP_DIR/next.log" 2>&1 &
WEB_PID="$!"

for _ in {1..120}; do
  if get_json "/api/gitmesh/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

web_issue_response="$(
  post_json \
    "/api/gitmesh/issues" \
    '{"repo":"farzeen/gitmesh","actor":"farzeen","title":"Web issue","body":"verifies api writes","labels":["web","collaboration"]}'
)"
expect_contains "$web_issue_response" '"ok":true'
expect_contains "$web_issue_response" '"number":"2"'

web_pr_response="$(
  post_json \
    "/api/gitmesh/pulls" \
    '{"repo":"farzeen/gitmesh","actor":"farzeen","sourceRef":"refs/heads/web-smoke","targetRef":"refs/heads/main","title":"Web pull request","body":"verifies api pr writes","labels":["web"]}'
)"
expect_contains "$web_pr_response" '"ok":true'
expect_contains "$web_pr_response" '"number":"2"'

web_issues_list="$(get_json "/api/gitmesh/issues?repo=farzeen/gitmesh")"
expect_contains "$web_issues_list" '"issues"'
expect_contains "$web_issues_list" '"title":"Web issue"'

web_pulls_list="$(get_json "/api/gitmesh/pulls?repo=farzeen/gitmesh")"
expect_contains "$web_pulls_list" '"pullRequests"'
expect_contains "$web_pulls_list" '"title":"Web pull request"'

web_status="$(get_json "/api/gitmesh/status")"
expect_contains "$web_status" '"ok":true'
expect_contains "$web_status" '"collaboration_events":"4"'

for store in "$OBJECT_STORE" "$REF_STORE" "$COLLAB_STORE"; do
  if [[ ! -s "$store" ]]; then
    echo "expected persisted store file: $store" >&2
    exit 1
  fi
done

echo "gitmesh local daemon smoke passed"
