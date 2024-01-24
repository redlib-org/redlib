#!/bin/bash

cd "$(dirname "$0")"
LATEST_TAG=$(curl -s https://api.github.com/repos/video-dev/hls.js/releases/latest | jq -r '.tag_name')

if [[ -z "$LATEST_TAG" || "$LATEST_TAG" == "null" ]]; then
  echo "Failed to fetch the latest release tag from GitHub."
  exit 1
fi

LICENSE="// @license http://www.apache.org/licenses/LICENSE-2.0 Apache-2.0
// @source  https://github.com/video-dev/hls.js/tree/$LATEST_TAG"

echo "$LICENSE" > ../static/hls.min.js

curl -s https://cdn.jsdelivr.net/npm/hls.js@${LATEST_TAG}/dist/hls.min.js >> ../static/hls.min.js

echo "Update complete. The latest hls.js (${LATEST_TAG}) has been saved to static/hls.min.js."
