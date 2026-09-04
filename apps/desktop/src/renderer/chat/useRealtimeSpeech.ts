import { useCallback, useEffect, useRef, useState } from "react";
import { speechApi } from "../api";

const PCM_BYTES_PER_SAMPLE = 2;

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
  const pendingSamplesRef = useRef<number[]>([]);
  const targetSampleRateRef = useRef(16_000);
  const chunkSamplesRef = useRef(3_200);
  const uploadChainRef = useRef<Promise<void>>(Promise.resolve());
  const recordingRef = useRef(false);
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
      pendingSamplesRef.current = [];
      voiceModeRef.current = false;
      setWaiting(false);
      setVoiceMode(false);
      onErrorRef.current(error instanceof Error ? error.message : String(error));
      if (sessionId) void speechApi.cancel(sessionId).catch(() => undefined);
    },
    [stopCapture],
  );

  const enqueuePcm = useCallback(
    (samples: number[]) => {
      const sessionId = sessionIdRef.current;
      if (!sessionId || samples.length === 0 || failedRef.current) return;
      const audioBase64 = pcm16Base64(samples);
      const upload = uploadChainRef.current.then(() =>
        speechApi.sendChunk(sessionId, audioBase64),
      );
      uploadChainRef.current = upload.then(
        () => undefined,
        (error) => fail(error),
      );
    },
    [fail],
  );

  const finishRecording = useCallback(async () => {
    if (!recordingRef.current) return;
    const sessionId = sessionIdRef.current;
    if (!sessionId) return;
    const remaining = pendingSamplesRef.current;
    pendingSamplesRef.current = [];
    if (remaining.length > 0) enqueuePcm(remaining);
    stopCapture();
    setWaiting(true);
    await uploadChainRef.current;
    if (!failedRef.current && sessionIdRef.current === sessionId) {
      try {
        await speechApi.finish(sessionId);
      } catch (error) {
        fail(error);
      }
    }
  }, [enqueuePcm, fail, stopCapture]);

  const beginRecording = useCallback(async () => {
    if (
      !enabled ||
      startingRef.current ||
      recordingRef.current ||
      waiting ||
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
      streamRef.current = stream;
      const session = await speechApi.start();
      sessionIdRef.current = session.sessionId;
      if (!mountedRef.current || !voiceModeRef.current) {
        for (const track of stream.getTracks()) track.stop();
        streamRef.current = undefined;
        sessionIdRef.current = undefined;
        await speechApi.cancel(session.sessionId).catch(() => undefined);
        return;
      }
      targetSampleRateRef.current = session.sampleRate;
      chunkSamplesRef.current = Math.max(
        1,
        Math.round((session.sampleRate * session.chunkMs) / 1_000),
      );
      pendingSamplesRef.current = [];
      uploadChainRef.current = Promise.resolve();
      failedRef.current = false;

      const context = new AudioContext();
      await context.resume();
      const source = context.createMediaStreamSource(stream);
      const processor = context.createScriptProcessor(4_096, 1, 1);
      contextRef.current = context;
      sourceRef.current = source;
      processorRef.current = processor;
      processor.onaudioprocess = (event) => {
        if (!recordingRef.current) return;
        const samples = downsample(
          event.inputBuffer.getChannelData(0),
          context.sampleRate,
          targetSampleRateRef.current,
        );
        pendingSamplesRef.current.push(...samples);
        while (pendingSamplesRef.current.length >= chunkSamplesRef.current) {
          enqueuePcm(
            pendingSamplesRef.current.splice(0, chunkSamplesRef.current),
          );
        }
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
  }, [enabled, enqueuePcm, fail, finishRecording, waiting]);

  const leaveVoiceMode = useCallback(() => {
    spaceDownRef.current = false;
    const sessionId = sessionIdRef.current;
    sessionIdRef.current = undefined;
    stopCapture();
    pendingSamplesRef.current = [];
    voiceModeRef.current = false;
    setWaiting(false);
    setVoiceMode(false);
    setTranscript("");
    if (sessionId) void speechApi.cancel(sessionId).catch(() => undefined);
  }, [stopCapture]);

  const enterVoiceMode = useCallback(() => {
    if (!enabled) return;
    failedRef.current = false;
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
