use std::fs;

use pretty_assertions::assert_eq;
use uuid::Uuid;

use super::TemporarySpeechRecording;

#[test]
fn saves_received_pcm_as_playable_wav() {
    let directory = tempfile::tempdir().unwrap();
    let session_id = format!("test-{}", Uuid::now_v7());
    let mut recording =
        TemporarySpeechRecording::create(directory.path(), &session_id, 16_000, 1).unwrap();
    recording.append(&[0x01, 0x02]).unwrap();
    recording.append(&[0x03, 0x04]).unwrap();
    let saved = recording.save().unwrap();

    let bytes = fs::read(&saved.path).unwrap();
    let mut expected = Vec::new();
    expected.extend_from_slice(b"RIFF");
    expected.extend_from_slice(&40_u32.to_le_bytes());
    expected.extend_from_slice(b"WAVEfmt ");
    expected.extend_from_slice(&16_u32.to_le_bytes());
    expected.extend_from_slice(&1_u16.to_le_bytes());
    expected.extend_from_slice(&1_u16.to_le_bytes());
    expected.extend_from_slice(&16_000_u32.to_le_bytes());
    expected.extend_from_slice(&32_000_u32.to_le_bytes());
    expected.extend_from_slice(&2_u16.to_le_bytes());
    expected.extend_from_slice(&16_u16.to_le_bytes());
    expected.extend_from_slice(b"data");
    expected.extend_from_slice(&4_u32.to_le_bytes());
    expected.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);
    assert_eq!(bytes, expected);
    assert_eq!(saved.read_pcm().unwrap(), vec![0x01, 0x02, 0x03, 0x04]);

    fs::remove_file(saved.path).unwrap();
}
