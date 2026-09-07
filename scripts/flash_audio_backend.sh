#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
backend="${1:-}"
port="${2:-/dev/ttyACM0}"
artifact_dir="$repo_root/target/audio-artifacts/$backend"

case "$backend" in
    open | esp-sr) ;;
    *)
        echo "usage: $0 open|esp-sr [serial-port]" >&2
        exit 2
        ;;
esac

for file in echo_byte.elf bootloader.bin partitions.csv; do
    if [[ ! -f "$artifact_dir/$file" ]]; then
        echo "missing $artifact_dir/$file; build the backend first" >&2
        exit 1
    fi
done

espflash flash \
    --port "$port" \
    --flash-size 32mb \
    --bootloader "$artifact_dir/bootloader.bin" \
    --partition-table "$artifact_dir/partitions.csv" \
    --partition-table-offset 0x8000 \
    --target-app-partition factory \
    "$artifact_dir/echo_byte.elf"

if [[ "$backend" == "esp-sr" ]]; then
    if [[ ! -f "$artifact_dir/srmodels.bin" ]]; then
        echo "missing $artifact_dir/srmodels.bin" >&2
        exit 1
    fi
    espflash write-bin --port "$port" 0x1220000 "$artifact_dir/srmodels.bin"
fi

echo "flashed $backend backend to $port"
