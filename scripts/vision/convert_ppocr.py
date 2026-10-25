"""Converts PP-OCRv4 ONNX models to RKNN for the RK3588 NPU.

det_<W>x<H>.rknn detection, INT8: 1920x448 strips (a 1080p screen is three
                 overlapping strips, one per NPU core) and 640x224 tiles for
                 small regions; the app picks the shape that costs least
rec_<W>.rknn     recognition, 48xW input, FP16; the app picks the narrowest
                 width that fits a line and splits longer lines at gaps

Both take uint8 NHWC input; the normalisation is folded into the model.

The model zoo's ONNX files have fixed input shapes (det 480x480, rec 48x320)
and rknn-toolkit2 ignores input_size_list for them, so the input shape is
rewritten in the ONNX graph before conversion.
"""

import argparse
import os
import sys
import tempfile

import onnx
from onnx import helper, shape_inference
from rknn.api import RKNN

DET_SHAPES = ((448, 1920), (224, 640))
REC_HEIGHT = 48
REC_WIDTHS = (320, 640, 1280)


def reshape_onnx(src, dst, height, width, output_4d=False):
    """Writes a copy of `src` whose single NCHW input is height x width.

    With `output_4d` the (N, T, C) output gets a unit axis, (N, T, 1, C).
    The NPU computes the recognizer head in that shape, and rknn-toolkit2
    otherwise appends a Reshape back to 3D that runs on the CPU.
    """
    model = onnx.load(src)
    if len(model.graph.input) != 1:
        sys.exit(f"{src}: expected one input, found {len(model.graph.input)}")
    dims = model.graph.input[0].type.tensor_type.shape.dim
    dims[2].ClearField("dim_param")
    dims[2].dim_value = height
    dims[3].ClearField("dim_param")
    dims[3].dim_value = width
    if output_4d:
        output = model.graph.output[0]
        # The models are opset 12, where Unsqueeze takes its axes as an attribute.
        model.graph.node.append(
            helper.make_node("Unsqueeze", [output.name], ["output_4d"], axes=[2])
        )
        output.name = "output_4d"
    # Drop the stale intermediate and output shapes so inference redoes them.
    del model.graph.value_info[:]
    for output in model.graph.output:
        output.type.tensor_type.ClearField("shape")
    model = shape_inference.infer_shapes(model)
    onnx.save(model, dst)


def build(onnx_path, out, input_size, mean, std, dataset=None, output_4d=False):
    _, _, height, width = input_size
    with tempfile.TemporaryDirectory() as tmp:
        reshaped = os.path.join(tmp, "model.onnx")
        reshape_onnx(onnx_path, reshaped, height, width, output_4d)
        rknn = RKNN(verbose=False)
        rknn.config(mean_values=[mean], std_values=[std], target_platform="rk3588")
        if rknn.load_onnx(model=reshaped) != 0:
            sys.exit(f"load {onnx_path} failed")
        if rknn.build(do_quantization=bool(dataset), dataset=dataset) != 0:
            sys.exit(f"build {out} failed")
        if rknn.export_rknn(out) != 0:
            sys.exit(f"export {out} failed")
        rknn.release()
    print(f"wrote {out} ({width}x{height})")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--det", required=True)
    parser.add_argument("--rec", required=True)
    parser.add_argument("--dataset", required=True, help="text file listing calibration images")
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    for height, width in DET_SHAPES:
        build(
            args.det,
            f"{args.out}/det_{width}x{height}.rknn",
            (1, 3, height, width),
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
            output_4d=True,
        )


if __name__ == "__main__":
    main()
