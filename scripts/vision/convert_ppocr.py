"""Converts PP-OCRv4 ONNX models to RKNN for the RK3588 NPU.

det.rknn         detection, 1088x1920 input (one 1080p screen per pass), INT8
rec_<W>.rknn     recognition, 48xW input, FP16; the app picks the narrowest
                 width that fits a line and splits longer lines at gaps

Both take uint8 NHWC input; the normalisation is folded into the model.
"""

import argparse
import sys

from rknn.api import RKNN

DET_SHAPE = (1088, 1920)
REC_HEIGHT = 48
REC_WIDTHS = (320, 640, 1280)


def build(onnx, out, input_size, mean, std, dataset=None):
    rknn = RKNN(verbose=False)
    config = dict(mean_values=[mean], std_values=[std], target_platform="rk3588")
    if dataset:
        config["quantized_algorithm"] = "mmse"
    rknn.config(**config)
    if rknn.load_onnx(model=onnx, input_size_list=[list(input_size)]) != 0:
        sys.exit(f"load {onnx} failed")
    if rknn.build(do_quantization=bool(dataset), dataset=dataset) != 0:
        sys.exit(f"build {out} failed")
    if rknn.export_rknn(out) != 0:
        sys.exit(f"export {out} failed")
    rknn.release()
    print(f"wrote {out}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--det", required=True)
    parser.add_argument("--rec", required=True)
    parser.add_argument("--dataset", required=True, help="text file listing calibration images")
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    build(
        args.det,
        f"{args.out}/det.rknn",
        (1, 3, *DET_SHAPE),
        [123.675, 116.28, 103.53],
        [58.395, 57.12, 57.375],
        dataset=args.dataset,
    )
    for width in REC_WIDTHS:
        build(
            args.rec,
            f"{args.out}/rec_{width}.rknn",
            (1, 3, REC_HEIGHT, width),
            [127.5, 127.5, 127.5],
            [127.5, 127.5, 127.5],
        )


if __name__ == "__main__":
    main()
