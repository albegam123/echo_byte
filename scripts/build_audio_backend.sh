#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

backend="${1:-}"
case "$backend" in
    open)
        cargo_args=(--release)
        ;;
    esp-sr)
        cargo_args=(--release --no-default-features --features audio-esp-sr)
        ;;
    *)
        echo "usage: $0 open|esp-sr" >&2
        exit 2
        ;;
esac

if [[ -z "${IDF_PATH:-}" && -f "$HOME/export-esp.sh" ]]; then
    # shellcheck disable=SC1091
    source "$HOME/export-esp.sh"
fi

if [[ -z "${IDF_PATH:-}" && -d "$repo_root/.embuild/espressif/esp-idf/v5.5.3" ]]; then
    export IDF_PATH="$repo_root/.embuild/espressif/esp-idf/v5.5.3"
fi
if [[ -z "${IDF_TOOLS_PATH:-}" && -d "$repo_root/.embuild/espressif" ]]; then
    export IDF_TOOLS_PATH="$repo_root/.embuild/espressif"
fi
export ESP_IDF_TOOLS_INSTALL_DIR="${ESP_IDF_TOOLS_INSTALL_DIR:-workspace}"
export IDF_GITHUB_ASSETS="${IDF_GITHUB_ASSETS:-dl.espressif.com/github_assets}"

cargo +esp build "${cargo_args[@]}"

artifact_dir="$repo_root/target/audio-artifacts/$backend"
mkdir -p "$artifact_dir"
cp "$repo_root/target/xtensa-esp32s3-espidf/release/echo_byte" \
    "$artifact_dir/echo_byte.elf"
cp "$repo_root/target/xtensa-esp32s3-espidf/release/bootloader.bin" \
    "$artifact_dir/bootloader.bin"
cp "$repo_root/target/xtensa-esp32s3-espidf/release/partition-table.bin" \
    "$artifact_dir/partition-table.bin"
cp "$repo_root/partitions.csv" "$artifact_dir/partitions.csv"

if [[ "$backend" == "esp-sr" ]]; then
    model_image="$(find "$repo_root/target/xtensa-esp32s3-espidf/release/build" \
        -path '*/out/build/srmodels/srmodels.bin' -print -quit)"
    if [[ -z "$model_image" ]]; then
        echo "ESP-SR model image was not generated" >&2
        exit 1
    fi
    cp "$model_image" "$artifact_dir/srmodels.bin"
fi

echo "built $backend backend: $artifact_dir"
if command -v xtensa-esp32s3-elf-size >/dev/null 2>&1; then
    xtensa-esp32s3-elf-size "$artifact_dir/echo_byte.elf"
fi
