use rustkvm::pipeline::H264Profile;
use rustkvm::webrtc::negotiated_h264_profile;
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters};

fn codec(mime_type: &str, fmtp: &str) -> RTCRtpCodecParameters {
    RTCRtpCodecParameters {
        capability: RTCRtpCodecCapability {
            mime_type: mime_type.to_owned(),
            clock_rate: 90_000,
            sdp_fmtp_line: fmtp.to_owned(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn h264(profile_level_id: &str) -> RTCRtpCodecParameters {
    codec(
        "video/H264",
        &format!(
            "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id={profile_level_id}"
        ),
    )
}

#[test]
fn profile_level_id_maps_to_encoder_profiles() {
    assert_eq!(H264Profile::from_profile_level_id("42e01f"), Some(H264Profile::Baseline));
    assert_eq!(H264Profile::from_profile_level_id("4d001f"), Some(H264Profile::Main));
    assert_eq!(H264Profile::from_profile_level_id("640034"), Some(H264Profile::High));
    assert_eq!(H264Profile::from_profile_level_id("f4001f"), None);
    assert_eq!(H264Profile::from_profile_level_id("x"), None);
}

#[test]
fn baseline_only_offer_negotiates_baseline() {
    // Chrome without a High-capable decoder, as captured from a device session.
    assert_eq!(negotiated_h264_profile(&[h264("42e01f")]), Some(H264Profile::Baseline));
}

#[test]
fn high_is_preferred_whenever_negotiated() {
    let codecs = [h264("42e01f"), h264("64001f")];
    assert_eq!(negotiated_h264_profile(&codecs), Some(H264Profile::High));
}

#[test]
fn constrained_high_falls_back_to_first_codec() {
    // 640c doesn't match the track's 6400, so binding takes the first codec.
    let codecs = [h264("42e01f"), h264("640c1f")];
    assert_eq!(negotiated_h264_profile(&codecs), Some(H264Profile::Baseline));
}

#[test]
fn high_needs_packetization_mode_1() {
    let codecs =
        [h264("42e01f"), codec("video/H264", "packetization-mode=0;profile-level-id=64001f")];
    assert_eq!(negotiated_h264_profile(&codecs), Some(H264Profile::Baseline));
}

#[test]
fn no_h264_negotiated() {
    assert_eq!(negotiated_h264_profile(&[codec("video/VP8", "")]), None);
}
