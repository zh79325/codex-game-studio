use std::io::Read;

use flate2::read::GzDecoder;
use pretty_assertions::assert_eq;
use serde_json::json;

use super::ServerFrame;
use super::audio_request;
use super::decode_server_frame;
use super::encode_request;

#[test]
fn audio_request_marks_the_last_packet() {
    let regular = audio_request(&[1, 2, 3], false).expect("regular audio frame");
    let final_packet = audio_request(&[], true).expect("final audio frame");

    assert_eq!(regular[0..4], [0x11, 0x20, 0x01, 0x00]);
    assert_eq!(final_packet[0..4], [0x11, 0x22, 0x01, 0x00]);

    let payload_size = u32::from_be_bytes(regular[4..8].try_into().expect("payload size"));
    let mut decoder = GzDecoder::new(&regular[8..]);
    let mut decoded = Vec::new();
    decoder.read_to_end(&mut decoded).expect("gzip payload");
    assert_eq!(payload_size as usize, regular.len() - 8);
    assert_eq!(decoded, vec![1, 2, 3]);
}

#[test]
fn decodes_final_transcript_response() {
    let payload = serde_json::to_vec(&json!({
        "result": {
            "text": "实时语音识别",
            "utterances": [{ "definite": true }]
        }
    }))
    .expect("json payload");
    let frame = encode_request([0x11, 0x93, 0x11, 0x00], &payload).expect("response frame");
    let mut frame_with_sequence = Vec::with_capacity(frame.len() + 4);
    frame_with_sequence.extend_from_slice(&frame[..4]);
    frame_with_sequence.extend_from_slice(&(-1_i32).to_be_bytes());
    frame_with_sequence.extend_from_slice(&frame[4..]);

    let decoded = decode_server_frame(&frame_with_sequence).expect("server frame");
    match decoded {
        ServerFrame::Transcript {
            text,
            definite,
            is_final,
        } => {
            assert_eq!(text, "实时语音识别");
            assert!(definite);
            assert!(is_final);
        }
        ServerFrame::Error(error) => panic!("unexpected error: {error}"),
        ServerFrame::Ignored => panic!("unexpected ignored frame"),
    }
}

#[test]
fn decodes_nostream_wrapped_final_response() {
    let payload = serde_json::to_vec(&json!({
        "code": 0,
        "is_last_package": true,
        "payload_msg": {
            "result": [
                { "text": "语音", "utterances": [{ "definite": true }] },
                { "text": "输入" }
            ]
        }
    }))
    .expect("json payload");
    let frame = encode_request([0x11, 0x90, 0x11, 0x00], &payload).expect("response frame");

    assert_eq!(
        decode_server_frame(&frame).expect("nostream response"),
        ServerFrame::Transcript {
            text: "语音输入".to_string(),
            definite: true,
            is_final: true,
        }
    );
}

#[test]
fn decodes_server_error() {
    let payload = br#"{"message":"invalid audio"}"#;
    let mut frame = vec![0x11, 0xf0, 0x10, 0x00];
    frame.extend_from_slice(&45000151_u32.to_be_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);

    match decode_server_frame(&frame).expect("server error frame") {
        ServerFrame::Error(error) => {
            assert!(error.contains("45000151"));
            assert!(error.contains("invalid audio"));
        }
        ServerFrame::Transcript { .. } => panic!("unexpected transcript"),
        ServerFrame::Ignored => panic!("unexpected ignored frame"),
    }
}
