#!/usr/bin/env bash
# Builds the on-device OCR bundle in out/vision/: PP-OCRv4 models converted
# for the RK3588 NPU, the character dictionary and Rockchip's librknnrt.
# Needs an x86_64 Linux host with Python 3.10-3.12 (rknn-toolkit2 has no
# wheels for newer Pythons). Install with: scp -O out/vision/* to
# /userdata/rustkvm/vision/ on the device (see docs/vision.md).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${OUT:-$ROOT/out/vision}
WORK=${WORK:-$ROOT/out/vision-work}
PYTHON=${PYTHON:-python3}

TOOLKIT_VERSION=2.5.0
RUNTIME_URL=https://raw.githubusercontent.com/airockchip/rknn-toolkit2/v$TOOLKIT_VERSION/rknpu2/runtime/Linux/librknn_api/aarch64/librknnrt.so
RUNTIME_SHA256=4b6cc575bedf8952b13db4b1190fa3b3d2ac213f00be774ff6ccd19d651970c0
LICENSE_URL=https://raw.githubusercontent.com/airockchip/rknn-toolkit2/v$TOOLKIT_VERSION/LICENSE
ZOO=https://raw.githubusercontent.com/airockchip/rknn_model_zoo/main
MODELS=https://ftrg.zbox.filez.com/v2/delivery/data/95f00b0fc900458ba134f8b180b3f7a1/examples/PPOCR
DET_SHA256=55a2092c7068bc8e9c79e4564958beaacdd66e802fd2eeffbba7604003dbb5c2
REC_SHA256=773fc4e13c25f4847fb82aa2112dc673355c13c0b32b066ae2fb082478810916
DICT_SHA256=28b2362ad4ab2dc38769aa72feb535e3a9ddb3fd2a7585a05920e6393b1dc7f7

fetch() { # url dest [sha256]
    if [ ! -s "$2" ]; then
        curl -fsSL --retry 3 -o "$2.part" "$1"
        mv "$2.part" "$2"
    fi
    if [ -n "${3:-}" ] && ! echo "$3  $2" | sha256sum -c --quiet -; then
        rm -f "$2"
        echo "checksum mismatch: $1" >&2
        exit 1
    fi
}

mkdir -p "$OUT" "$WORK/dataset"
fetch "$MODELS/ppocrv4_det.onnx" "$WORK/ppocrv4_det.onnx" "$DET_SHA256"
fetch "$MODELS/ppocrv4_rec.onnx" "$WORK/ppocrv4_rec.onnx" "$REC_SHA256"
fetch "$ZOO/examples/PPOCR/PPOCR-Rec/model/ppocr_keys_v1.txt" "$OUT/ppocr_keys_v1.txt" "$DICT_SHA256"
fetch "$RUNTIME_URL" "$OUT/librknnrt.so" "$RUNTIME_SHA256"
fetch "$LICENSE_URL" "$OUT/LICENSE.librknnrt"

fetch "$ZOO/datasets/PPOCR/imgs/dataset_20.txt" "$WORK/dataset/list.txt"
while read -r img; do
    [ -n "$img" ] && fetch "$ZOO/datasets/PPOCR/imgs/${img#./}" "$WORK/dataset/${img#./}"
done < "$WORK/dataset/list.txt"
sed 's#^\./#'"$WORK"'/dataset/#' "$WORK/dataset/list.txt" > "$WORK/dataset/abs.txt"

if [ ! -e "$WORK/venv/.toolkit-$TOOLKIT_VERSION" ]; then
    "$PYTHON" -m venv "$WORK/venv"
    # rknn-toolkit2 still imports pkg_resources, which setuptools 81 removed.
    "$WORK/venv/bin/pip" install --quiet "rknn-toolkit2==$TOOLKIT_VERSION" "setuptools<81"
    touch "$WORK/venv/.toolkit-$TOOLKIT_VERSION"
fi
"$WORK/venv/bin/python" "$ROOT/scripts/vision/convert_ppocr.py" \
    --det "$WORK/ppocrv4_det.onnx" --rec "$WORK/ppocrv4_rec.onnx" \
    --dataset "$WORK/dataset/abs.txt" --out "$OUT"

ls -l "$OUT"
