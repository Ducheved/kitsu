#!/usr/bin/env bash
# Hand the Apple signing and notarization secrets that exist to `tauri build`
# (through GITHUB_ENV), and only those: Tauri treats a variable that is set
# but empty as configured and fails on it. No certificate means an ad-hoc
# signature ("-"), which Apple Silicon requires to launch the app at all.
#
# Signing:       APPLE_CERTIFICATE (base64 .p12), APPLE_CERTIFICATE_PASSWORD,
#                APPLE_SIGNING_IDENTITY (optional, taken from the certificate)
# Notarization:  APPLE_ID, APPLE_PASSWORD (app-specific), APPLE_TEAM_ID
#           or:  APPLE_API_KEY (key id), APPLE_API_ISSUER, APPLE_API_KEY_P8
#                (contents of AuthKey_<id>.p8)
set -euo pipefail

export_var() {
  local name=$1 value=$2 delim
  delim="EOF_$(openssl rand -hex 16)"
  printf '%s<<%s\n%s\n%s\n' "$name" "$delim" "$value" "$delim" >>"${GITHUB_ENV:?}"
}

if [[ -n "${APPLE_CERTIFICATE:-}" ]]; then
  if [[ -z "${APPLE_CERTIFICATE_PASSWORD:-}" ]]; then
    echo "::error::APPLE_CERTIFICATE is set but APPLE_CERTIFICATE_PASSWORD is not"
    exit 1
  fi
  export_var APPLE_CERTIFICATE "$APPLE_CERTIFICATE"
  export_var APPLE_CERTIFICATE_PASSWORD "$APPLE_CERTIFICATE_PASSWORD"
  if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
    export_var APPLE_SIGNING_IDENTITY "$APPLE_SIGNING_IDENTITY"
  fi
  echo "macOS: signing with the Developer ID certificate"

  if [[ -n "${APPLE_ID:-}" && -n "${APPLE_PASSWORD:-}" && -n "${APPLE_TEAM_ID:-}" ]]; then
    export_var APPLE_ID "$APPLE_ID"
    export_var APPLE_PASSWORD "$APPLE_PASSWORD"
    export_var APPLE_TEAM_ID "$APPLE_TEAM_ID"
    echo "macOS: notarizing with an Apple ID"
  elif [[ -n "${APPLE_API_KEY:-}" && -n "${APPLE_API_ISSUER:-}" && -n "${APPLE_API_KEY_P8:-}" ]]; then
    key_path="${RUNNER_TEMP:?}/AuthKey_$APPLE_API_KEY.p8"
    printf '%s\n' "$APPLE_API_KEY_P8" >"$key_path"
    export_var APPLE_API_KEY "$APPLE_API_KEY"
    export_var APPLE_API_ISSUER "$APPLE_API_ISSUER"
    export_var APPLE_API_KEY_PATH "$key_path"
    echo "macOS: notarizing with an App Store Connect API key"
  else
    echo "::warning::macOS: signed but not notarized (no APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID or APPLE_API_* secrets)"
  fi
else
  export_var APPLE_SIGNING_IDENTITY "-"
  echo "macOS: no APPLE_CERTIFICATE secret, ad-hoc signature, not notarized"
fi
