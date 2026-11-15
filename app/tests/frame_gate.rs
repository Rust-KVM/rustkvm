use rustkvm::pipeline::{FrameGate, contains_h264_idr};

const IDR: &[u8] = &[0, 0, 0, 1, 0x67, 0x64, 0, 0, 0, 1, 0x68, 0, 0, 1, 0x65, 0x88];
const P_FRAME: &[u8] = &[0, 0, 0, 1, 0x41, 0x9a];

#[test]
fn idr_detection_reads_the_first_slice() {
    assert!(contains_h264_idr(IDR));
    assert!(!contains_h264_idr(P_FRAME));
    assert!(!contains_h264_idr(&[]));
}

#[test]
fn frames_pass_until_one_is_dropped() {
    let mut gate = FrameGate::default();
    assert!(gate.admit(|| panic!("keyframe check is not needed while the stream is intact")));
    assert!(gate.admit(|| false));
}

#[test]
fn after_a_drop_only_a_keyframe_reopens_the_gate() {
    let mut gate = FrameGate::default();
    assert!(gate.frame_dropped(), "the first drop asks for a keyframe");

    assert!(!gate.admit(|| contains_h264_idr(P_FRAME)));
    assert!(!gate.admit(|| contains_h264_idr(P_FRAME)));
    assert!(gate.admit(|| contains_h264_idr(IDR)));
    assert!(gate.admit(|| contains_h264_idr(P_FRAME)));
}

#[test]
fn one_keyframe_request_per_gap() {
    let mut gate = FrameGate::default();
    assert!(gate.frame_dropped());
    assert!(!gate.frame_dropped(), "already waiting for the keyframe");

    // The keyframe itself is dropped: ask again.
    assert!(gate.admit(|| true));
    assert!(gate.frame_dropped());
}
