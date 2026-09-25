/** Convert decoded browser audio to the mono 16 kHz PCM16 WAV expected by whisper.cpp. */
export function audioBufferToWav16k(audio: AudioBuffer): Blob {
  const channels = Array.from({ length: audio.numberOfChannels }, (_, index) => audio.getChannelData(index));
  const sourceRate = audio.sampleRate;
  const targetRate = 16_000;
  const outputLength = Math.max(1, Math.ceil(audio.length * targetRate / sourceRate));
  const samples = new Int16Array(outputLength);

  // Linear interpolation also handles the common 44.1/48 kHz to 16 kHz conversion.
  for (let i = 0; i < outputLength; i++) {
    const position = i * sourceRate / targetRate;
    const left = Math.min(Math.floor(position), audio.length - 1);
    const right = Math.min(left + 1, audio.length - 1);
    const fraction = position - left;
    let value = 0;
    for (const channel of channels) value += channel[left] * (1 - fraction) + channel[right] * fraction;
    value = Math.max(-1, Math.min(1, value / channels.length));
    samples[i] = value < 0 ? Math.round(value * 0x8000) : Math.round(value * 0x7fff);
  }

  const wav = new ArrayBuffer(44 + samples.byteLength);
  const view = new DataView(wav);
  const write = (offset: number, value: string) => {
    for (let i = 0; i < value.length; i++) view.setUint8(offset + i, value.charCodeAt(i));
  };
  write(0, 'RIFF');
  view.setUint32(4, 36 + samples.byteLength, true);
  write(8, 'WAVE');
  write(12, 'fmt ');
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, 1, true); // mono
  view.setUint32(24, targetRate, true);
  view.setUint32(28, targetRate * 2, true);
  view.setUint16(32, 2, true); // block align
  view.setUint16(34, 16, true);
  write(36, 'data');
  view.setUint32(40, samples.byteLength, true);
  new Int16Array(wav, 44).set(samples);
  return new Blob([wav], { type: 'audio/wav' });
}
