use std::io::Read;
use std::io::Write;

use codex_game_app_server_adapter::RealtimeSpeechRoute;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde_json::Value;
use serde_json::json;

pub(super) enum ServerFrame {
    Transcript {
        text: String,
        definite: bool,
        is_final: bool,
    },
    Error(String),
    Ignored,
}

pub(super) fn full_client_request(route: &RealtimeSpeechRoute) -> Result<Vec<u8>, String> {
    let payload = serde_json::to_vec(&json!({
        "user": { "uid": "codex-game-studio" },
        "audio": {
            "format": "pcm",
            "codec": "raw",
            "rate": route.sample_rate,
            "bits": 16,
            "channel": route.channels
        },
        "request": {
            "model_name": route.model,
            "enable_nonstream": true,
            "enable_itn": true,
            "enable_punc": true,
            "show_utterances": true,
            "result_type": "full",
            "end_window_size": 800
        }
    }))
    .map_err(|error| error.to_string())?;
    encode_request([0x11, 0x10, 0x11, 0x00], &payload)
}

pub(super) fn audio_request(audio: &[u8], is_last: bool) -> Result<Vec<u8>, String> {
    encode_request(
        [0x11, if is_last { 0x22 } else { 0x20 }, 0x01, 0x00],
        audio,
    )
}

fn encode_request(header: [u8; 4], payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(payload)
        .map_err(|error| format!("压缩语音请求失败：{error}"))?;
    let payload = encoder
        .finish()
        .map_err(|error| format!("压缩语音请求失败：{error}"))?;
    let payload_size = u32::try_from(payload.len()).map_err(|_| "语音请求过大".to_string())?;
    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.extend_from_slice(&header);
    frame.extend_from_slice(&payload_size.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub(super) fn decode_server_frame(frame: &[u8]) -> Result<ServerFrame, String> {
    if frame.len() < 8 {
        return Err("语音识别服务返回了不完整的数据帧".to_string());
    }
    let header_size = usize::from(frame[0] & 0x0f) * 4;
    let message_type = frame[1] >> 4;
    let flags = frame[1] & 0x0f;
    let compression = frame[2] & 0x0f;
    if header_size < 4 || frame.len() < header_size {
        return Err("语音识别服务返回了无效的数据帧头".to_string());
    }
    let mut offset = header_size;
    if message_type == 0x0f {
        let error_code = read_u32(frame, offset)?;
        offset += 4;
        let payload = read_payload(frame, offset, compression)?;
        return Ok(ServerFrame::Error(format!(
            "语音识别服务错误 {error_code}：{}",
            String::from_utf8_lossy(&payload)
        )));
    }
    if message_type != 0x09 {
        return Ok(ServerFrame::Ignored);
    }
    if flags & 0x01 != 0 {
        read_i32(frame, offset)?;
        offset += 4;
    }
    let payload = read_payload(frame, offset, compression)?;
    let value: Value = serde_json::from_slice(&payload)
        .map_err(|error| format!("语音识别结果格式错误：{error}"))?;
    let result = value.get("result").and_then(|result| {
        result
            .as_array()
            .and_then(|items| items.last())
            .or(Some(result))
    });
    let text = result
        .and_then(|result| result.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let definite = result
        .and_then(|result| result.get("utterances"))
        .and_then(Value::as_array)
        .is_some_and(|utterances| {
            utterances
                .iter()
                .any(|utterance| utterance.get("definite").and_then(Value::as_bool) == Some(true))
        });
    Ok(ServerFrame::Transcript {
        text,
        definite,
        is_final: flags & 0x02 != 0,
    })
}

fn read_payload(frame: &[u8], offset: usize, compression: u8) -> Result<Vec<u8>, String> {
    let payload_size = read_u32(frame, offset)? as usize;
    let payload_start = offset + 4;
    let payload_end = payload_start
        .checked_add(payload_size)
        .ok_or_else(|| "语音识别响应长度溢出".to_string())?;
    let payload = frame
        .get(payload_start..payload_end)
        .ok_or_else(|| "语音识别响应负载不完整".to_string())?;
    match compression {
        0 => Ok(payload.to_vec()),
        1 => {
            let mut decoder = GzDecoder::new(payload);
            let mut decoded = Vec::new();
            decoder
                .read_to_end(&mut decoded)
                .map_err(|error| format!("解压语音识别响应失败：{error}"))?;
            Ok(decoded)
        }
        other => Err(format!("不支持的语音识别响应压缩格式：{other}")),
    }
}

fn read_u32(frame: &[u8], offset: usize) -> Result<u32, String> {
    let bytes: [u8; 4] = frame
        .get(offset..offset + 4)
        .ok_or_else(|| "语音识别响应字段不完整".to_string())?
        .try_into()
        .map_err(|_| "语音识别响应字段长度无效".to_string())?;
    Ok(u32::from_be_bytes(bytes))
}

fn read_i32(frame: &[u8], offset: usize) -> Result<i32, String> {
    let bytes: [u8; 4] = frame
        .get(offset..offset + 4)
        .ok_or_else(|| "语音识别响应序号不完整".to_string())?
        .try_into()
        .map_err(|_| "语音识别响应序号长度无效".to_string())?;
    Ok(i32::from_be_bytes(bytes))
}

#[cfg(test)]
#[path = "game_speech_protocol_tests.rs"]
mod tests;
