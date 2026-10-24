#!/usr/bin/env bash
# Builds the Dioxus web app into ./dist (static files served by Vercel).
# Needs SUPABASE_URL and SUPABASE_PUBLISHABLE_KEY (Project Settings → Environment Variables).
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"

: "${SUPABASE_URL:?Set SUPABASE_URL in the Vercel project settings}"
: "${SUPABASE_PUBLISHABLE_KEY:?Set SUPABASE_PUBLISHABLE_KEY in the Vercel project settings}"

rm -rf dist
dx bundle --web --release --package web --out-dir dist-build
# dx puts the static site in a `public` folder inside the output directory.
if [ -d dist-build/public ]; then mv dist-build/public dist; else mv dist-build dist; fi
rm -rf dist-build
ls -la dist
