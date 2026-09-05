use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

const WAV_HEADER_BYTES: u64 = 44;
const PCM_BITS_PER_SAMPLE: u16 = 16;
const PCM_BYTES_PER_SAMPLE: u16 = PCM_BITS_PER_SAMPLE / 8;

pub(super) struct TemporarySpeechRecording {
    file: File,
    path: PathBuf,
    sample_rate: u32,
    channels: u16,
    data_bytes: u32,
}

pub(super) struct SavedSpeechRecording {
    pub(super) path: PathBuf,
    pub(super) data_bytes: u64,
}

impl TemporarySpeechRecording {
    pub(super) fn create(
        directory: &Path,
        session_id: &str,
        sample_rate: u32,
        channels: u16,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(directory)
            .map_err(|error| format!("创建临时录音目录失败：{error}"))?;
        let path = directory.join(format!("{session_id}.wav"));
        let mut file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| format!("创建临时录音文件失败：{error}"))?;
        write_wav_header(&mut file, sample_rate, channels, 0)?;
        Ok(Self {
            file,
            path,
            sample_rate,
            channels,
            data_bytes: 0,
        })
    }

    pub(super) fn path(&self) -> &PathBuf {
        &self.path
    }

    pub(super) fn append(&mut self, audio: &[u8]) -> Result<(), String> {
        let additional_bytes = u32::try_from(audio.len())
            .map_err(|_| "单个语音分片过大，无法写入 WAV 文件".to_string())?;
        self.data_bytes = self
            .data_bytes
            .checked_add(additional_bytes)
            .ok_or_else(|| "录音过长，无法写入 WAV 文件".to_string())?;
        self.file
            .write_all(audio)
            .map_err(|error| format!("写入临时录音文件失败：{error}"))
    }

    pub(super) fn save(mut self) -> Result<SavedSpeechRecording, String> {
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|error| format!("更新临时录音文件失败：{error}"))?;
        write_wav_header(
            &mut self.file,
            self.sample_rate,
            self.channels,
            self.data_bytes,
        )?;
        self.file
            .flush()
            .map_err(|error| format!("保存临时录音文件失败：{error}"))?;
        Ok(SavedSpeechRecording {
            path: self.path,
            data_bytes: u64::from(self.data_bytes),
        })
    }
}

impl SavedSpeechRecording {
    pub(super) fn read_pcm(&self) -> Result<Vec<u8>, String> {
        let mut file =
            File::open(&self.path).map_err(|error| format!("打开临时录音文件失败：{error}"))?;
        file.seek(SeekFrom::Start(WAV_HEADER_BYTES))
            .map_err(|error| format!("读取临时录音文件失败：{error}"))?;
        let mut audio = Vec::with_capacity(self.data_bytes as usize);
        file.take(self.data_bytes)
            .read_to_end(&mut audio)
            .map_err(|error| format!("读取临时录音文件失败：{error}"))?;
        Ok(audio)
    }
}

fn write_wav_header(
    file: &mut File,
    sample_rate: u32,
    channels: u16,
    data_bytes: u32,
) -> Result<(), String> {
    let block_align = channels
        .checked_mul(PCM_BYTES_PER_SAMPLE)
        .ok_or_else(|| "WAV 声道数无效".to_string())?;
    let byte_rate = sample_rate
        .checked_mul(u32::from(block_align))
        .ok_or_else(|| "WAV 采样率无效".to_string())?;
    let riff_size = 36_u32
        .checked_add(data_bytes)
        .ok_or_else(|| "录音过长，无法写入 WAV 文件".to_string())?;
    let mut header = Vec::with_capacity(WAV_HEADER_BYTES as usize);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&riff_size.to_le_bytes());
    header.extend_from_slice(b"WAVE");
    header.extend_from_slice(b"fmt ");
    header.extend_from_slice(&16_u32.to_le_bytes());
    header.extend_from_slice(&1_u16.to_le_bytes());
    header.extend_from_slice(&channels.to_le_bytes());
    header.extend_from_slice(&sample_rate.to_le_bytes());
    header.extend_from_slice(&byte_rate.to_le_bytes());
    header.extend_from_slice(&block_align.to_le_bytes());
    header.extend_from_slice(&PCM_BITS_PER_SAMPLE.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data_bytes.to_le_bytes());
    file.write_all(&header)
        .map_err(|error| format!("写入 WAV 文件头失败：{error}"))
}

#[cfg(test)]
#[path = "game_speech_recording_tests.rs"]
mod tests;
