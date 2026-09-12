#!/usr/bin/env bash
set -euo pipefail

repo_dir="/root/zhihua-service"
repo_url="https://github.com/woniu101/zhihua-service.git"

if [[ -d "$repo_dir/.git" ]]; then
  git -C "$repo_dir" fetch --depth 1 origin main
  git -C "$repo_dir" reset --hard origin/main
else
  if [[ -e "$repo_dir" ]]; then
    echo "Refusing to replace an existing non-Git directory: $repo_dir" >&2
    exit 1
  fi
  git clone --depth 1 --branch main "$repo_url" "$repo_dir"
fi

cd "$repo_dir"
python_bin="/root/miniconda3/bin/python"
if [[ ! -x "$python_bin" ]]; then
  python_bin="$(command -v python3)"
fi
if [[ ! -x .venv/bin/python ]]; then
  "$python_bin" -m venv .venv
fi
.venv/bin/python -m pip install --disable-pip-version-check -q -e '.[dev]'

if [[ ! -f .env ]]; then
  cp .env.example .env
  token="$(.venv/bin/python -c 'import secrets; print(secrets.token_urlsafe(48))')"
  sed -i "s|^ZHIHUA_SERVICE_TOKEN=.*|ZHIHUA_SERVICE_TOKEN=$token|" .env
fi
chmod 600 .env
mkdir -p /root/zhihua-service-data

if command -v sudo >/dev/null 2>&1; then
  sudo bash deploy/install-supervisor.sh "$repo_dir"
else
  bash deploy/install-supervisor.sh "$repo_dir"
fi
.venv/bin/python -m pytest -q
.venv/bin/python -m ruff check .
echo deployment-complete
