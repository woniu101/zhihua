#!/usr/bin/env bash
set -euo pipefail

comfyui_dir="${COMFYUI_ROOT:-/root/ComfyUI}"
python_bin="${COMFYUI_PYTHON:-/root/miniconda3/bin/python}"

mapfile -t pids < <(pgrep -f 'python(3)? -u main.py .*--port 8188' || true)
if (( ${#pids[@]} > 0 )); then
  kill "${pids[@]}"
  for _ in {1..20}; do
    if ! kill -0 "${pids[0]}" 2>/dev/null; then
      break
    fi
    sleep 0.5
  done
fi

cd "$comfyui_dir"
nohup "$python_bin" -u main.py --listen 0.0.0.0 --port 8188 --enable-manager \
  >/root/comfyui.log 2>&1 </dev/null &

for _ in {1..120}; do
  if "$python_bin" -c \
    'import urllib.request; urllib.request.urlopen("http://127.0.0.1:8188/system_stats", timeout=2)' \
    >/dev/null 2>&1; then
    echo comfyui-ready
    exit 0
  fi
  sleep 1
done

tail -80 /root/comfyui.log >&2 || true
echo "ComfyUI did not become ready" >&2
exit 1
