import { useCallback, useEffect, useRef, useState } from "react";
import { speechApi } from "../api";

const PCM_BYTES_PER_SAMPLE = 2;
const TARGET_SAMPLE_RATE = 16_000;

type SpeechNotification = {
  method: string;
  params?: Record<string, unknown>;
};

type UseRealtimeSpeechOptions = {
  enabled: boolean;
  onCompleted: (text: string) => void;
  onError: (message: string) => void;
};

export function useRealtimeSpeech({
  enabled,
  onCompleted,
  onError,
}: UseRealtimeSpeechOptions) {
  const [voiceMode, setVoiceMode] = useState(false);
  const [recording, setRecording] = useState(false);
  const [waiting, setWaiting] = useState(false);
  const [transcript, setTranscript] = useState("");
  const sessionIdRef = useRef<string | undefined>(undefined);
  const streamRef = useRef<MediaStream | undefined>(undefined);
  const contextRef = useRef<AudioContext | undefined>(undefined);
  const sourceRef = useRef<MediaStreamAudioSourceNode | undefined>(undefined);
  const processorRef = useRef<ScriptProcessorNode | undefined>(undefined);
  const recordedSamplesRef = useRef<number[]>([]);
  const captureSampleRateRef = useRef(TARGET_SAMPLE_RATE);
  const recordingRef = useRef(false);
  const waitingRef = useRef(false);
  const startingRef = useRef(false);
  const spaceDownRef = useRef(false);
  const failedRef = useRef(false);
  const voiceModeRef = useRef(false);
  const mountedRef = useRef(true);
  const onCompletedRef = useRef(onCompleted);
  const onErrorRef = useRef(onError);

  useEffect(() => {
    onCompletedRef.current = onCompleted;
    onErrorRef.current = onError;
  }, [onCompleted, onError]);

  const stopCapture = useCallback(() => {
    recordingRef.current = false;
    processorRef.current?.disconnect();
    sourceRef.current?.disconnect();
    for (const track of streamRef.current?.getTracks() ?? []) track.stop();
    void contextRef.current?.close();
    processorRef.current = undefined;
    sourceRef.current = undefined;
    streamRef.current = undefined;
    contextRef.current = undefined;
    setRecording(false);
  }, []);

  const fail = useCallback(
    (error: unknown) => {
      if (failedRef.current) return;
      failedRef.current = true;
      const sessionId = sessionIdRef.current;
      sessionIdRef.current = undefined;
      stopCapture();
      recordedSamplesRef.current = [];
      voiceModeRef.current = false;
      waitingRef.current = false;
      setWaiting(false);
      setVoiceMode(false);
      onErrorRef.current(error instanceof Error ? error.message : String(error));
      if (sessionId) void speechApi.cancel(sessionId).catch(() => undefined);
    },
    [stopCapture],
  );

  const finishRecording = useCallback(async () => {
    if (!recordingRef.current) return;
    const samples = recordedSamplesRef.current;
    recordedSamplesRef.current = [];
    stopCapture();
    if (samples.length === 0) return;
    waitingRef.current = true;
    setWaiting(true);
    try {
      const session = await speechApi.start();
      sessionIdRef.current = session.sessionId;
      if (!mountedRef.current || !voiceModeRef.current) {
        sessionIdRef.current = undefined;
        await speechApi.cancel(session.sessionId).catch(() => undefined);
        return;
      }
      if (session.channels !== 1) {
        throw new Error(`暂不支持 ${session.channels} 声道语音识别`);
      }
      const pcm = downsample(
        new Float32Array(samples),
        captureSampleRateRef.current,
        session.sampleRate,
      );
      const chunkSamples = Math.max(
        1,
        Math.round((session.sampleRate * session.chunkMs) / 1_000),
      );
      for (let offset = 0; offset < pcm.length; offset += chunkSamples) {
        if (
          !voiceModeRef.current ||
          sessionIdRef.current !== session.sessionId
        ) {
          return;
        }
        await speechApi.sendChunk(
          session.sessionId,
          pcm16Base64(pcm.slice(offset, offset + chunkSamples)),
        );
        if (offset + chunkSamples < pcm.length) {
          await new Promise((resolve) => window.setTimeout(resolve, session.chunkMs));
        }
      }
      if (
        voiceModeRef.current &&
        sessionIdRef.current === session.sessionId
      ) {
        await speechApi.finish(session.sessionId);
      }
    } catch (error) {
      fail(error);
    }
  }, [fail, stopCapture]);

  const beginRecording = useCallback(async () => {
    if (
      !enabled ||
      startingRef.current ||
      recordingRef.current ||
      waitingRef.current ||
      failedRef.current
    ) {
      return;
    }
    startingRef.current = true;
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
        },
      });
      if (!mountedRef.current || !voiceModeRef.current) {
        for (const track of stream.getTracks()) track.stop();
        return;
      }
      streamRef.current = stream;
      recordedSamplesRef.current = [];
      failedRef.current = false;

      const context = new AudioContext();
      contextRef.current = context;
      await context.resume();
      if (!mountedRef.current || !voiceModeRef.current) {
        stopCapture();
        return;
      }
      const source = context.createMediaStreamSource(stream);
      const processor = context.createScriptProcessor(4_096, 1, 1);
      captureSampleRateRef.current = context.sampleRate;
      sourceRef.current = source;
      processorRef.current = processor;
      processor.onaudioprocess = (event) => {
        if (!recordingRef.current) return;
        const samples = event.inputBuffer.getChannelData(0);
        for (const sample of samples) recordedSamplesRef.current.push(sample);
      };
      source.connect(processor);
      processor.connect(context.destination);
      recordingRef.current = true;
      setRecording(true);
      setTranscript("");
      if (!spaceDownRef.current) void finishRecording();
    } catch (error) {
      fail(error);
    } finally {
      startingRef.current = false;
    }
  }, [enabled, fail, finishRecording, stopCapture]);

  const leaveVoiceMode = useCallback(() => {
    spaceDownRef.current = false;
    const sessionId = sessionIdRef.current;
    sessionIdRef.current = undefined;
    stopCapture();
    recordedSamplesRef.current = [];
    voiceModeRef.current = false;
    waitingRef.current = false;
    setWaiting(false);
    setVoiceMode(false);
    setTranscript("");
    if (sessionId) void speechApi.cancel(sessionId).catch(() => undefined);
  }, [stopCapture]);

  const enterVoiceMode = useCallback(() => {
    if (!enabled) return;
    failedRef.current = false;
    waitingRef.current = false;
    voiceModeRef.current = true;
    setTranscript("");
    setVoiceMode(true);
  }, [enabled]);

  useEffect(() => {
    if (!enabled && voiceModeRef.current) leaveVoiceMode();
  }, [enabled, leaveVoiceMode]);

  useEffect(() => {
    if (!voiceMode) return;
    const keyDown = (event: KeyboardEvent) => {
      if (event.code !== "Space" || event.repeat) return;
      event.preventDefault();
      spaceDownRef.current = true;
      void beginRecording();
    };
    const keyUp = (event: KeyboardEvent) => {
      if (event.code !== "Space") return;
      event.preventDefault();
      spaceDownRef.current = false;
      void finishRecording();
    };
    const blur = () => {
      if (!spaceDownRef.current) return;
      spaceDownRef.current = false;
      void finishRecording();
    };
    window.addEventListener("keydown", keyDown);
    window.addEventListener("keyup", keyUp);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", keyDown);
      window.removeEventListener("keyup", keyUp);
      window.removeEventListener("blur", blur);
    };
  }, [beginRecording, finishRecording, voiceMode]);

  useEffect(
    () =>
      window.codexGame.onEvent((event) => {
        if (typeof event !== "object" || !event || !("method" in event)) return;
        const notification = event as SpeechNotification;
        const params = notification.params ?? {};
        if (params.sessionId !== sessionIdRef.current) return;
        if (
          notification.method === "game/speech/transcript" &&
          typeof params.text === "string"
        ) {
          setTranscript(params.text);
          return;
        }
        if (notification.method === "game/speech/completed") {
          const text = typeof params.text === "string" ? params.text.trim() : "";
          sessionIdRef.current = undefined;
          voiceModeRef.current = false;
          waitingRef.current = false;
          setWaiting(false);
          setVoiceMode(false);
          setTranscript("");
          if (text) onCompletedRef.current(text);
          return;
        }
        if (
          notification.method === "game/speech/error" &&
          typeof params.message === "string"
        ) {
          fail(params.message);
        }
      }),
    [fail],
  );

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      voiceModeRef.current = false;
      waitingRef.current = false;
      const sessionId = sessionIdRef.current;
      sessionIdRef.current = undefined;
      stopCapture();
      if (sessionId) void speechApi.cancel(sessionId).catch(() => undefined);
    };
  }, [stopCapture]);

  return {
    voiceMode,
    recording,
    waiting,
    transcript,
    enterVoiceMode,
    leaveVoiceMode,
  };
}

function downsample(
  input: Float32Array,
  inputSampleRate: number,
  outputSampleRate: number,
) {
  if (inputSampleRate === outputSampleRate) return Array.from(input);
  const ratio = inputSampleRate / outputSampleRate;
  const outputLength = Math.max(1, Math.round(input.length / ratio));
  const output = new Array<number>(outputLength);
  for (let index = 0; index < outputLength; index += 1) {
    const sourceIndex = Math.min(input.length - 1, Math.floor(index * ratio));
    output[index] = input[sourceIndex];
  }
  return output;
}

function pcm16Base64(samples: number[]) {
  const bytes = new Uint8Array(samples.length * PCM_BYTES_PER_SAMPLE);
  const view = new DataView(bytes.buffer);
  for (let index = 0; index < samples.length; index += 1) {
    const sample = Math.max(-1, Math.min(1, samples[index]));
    view.setInt16(
      index * PCM_BYTES_PER_SAMPLE,
      sample < 0 ? sample * 0x8000 : sample * 0x7fff,
      true,
    );
  }
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}
