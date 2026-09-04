const SAMPLE_BATCH_SIZE = 4_096;

class SpeechRecorderProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.samples = new Float32Array(SAMPLE_BATCH_SIZE);
    this.sampleCount = 0;
    this.capturing = true;
    this.port.onmessage = (event) => {
      if (event.data?.type !== "flush") return;
      this.capturing = false;
      this.flushSamples();
      this.port.postMessage({ type: "flushed" });
    };
  }

  process(inputs) {
    if (!this.capturing) return true;
    const input = inputs[0]?.[0];
    if (!input) return true;
    for (const sample of input) {
      this.samples[this.sampleCount] = sample;
      this.sampleCount += 1;
      if (this.sampleCount === this.samples.length) this.flushSamples();
    }
    return true;
  }

  flushSamples() {
    if (this.sampleCount === 0) return;
    const samples = this.samples.slice(0, this.sampleCount);
    this.port.postMessage({ type: "samples", samples }, [samples.buffer]);
    this.samples = new Float32Array(SAMPLE_BATCH_SIZE);
    this.sampleCount = 0;
  }
}

registerProcessor("speech-recorder", SpeechRecorderProcessor);
