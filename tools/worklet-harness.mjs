// Runs the JS engine's AudioWorklet (src/formant-worklet.js) in Node, the way
// a browser's audio rendering thread does, for the player parity tests.
//
// A scenario is { sr, outputs, processorOptions, ops }:
//   outputs  channel count of each worklet output (0 = an output with no channels)
//   ops      { op: 'message', data } delivered to port.onmessage between
//            render quanta, or { op: 'process', quanta } rendering 128-frame blocks
// `currentTime` is frames rendered / sampleRate, as in a real AudioWorkletGlobalScope.

import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

export const QUANTUM = 128;

export async function loadWorklet(jsRoot) {
  let Processor = null;
  globalThis.AudioWorkletProcessor = class {
    constructor() { this.port = { onmessage: null, postMessage() {} }; }
  };
  globalThis.registerProcessor = (name, cls) => { if (name === 'formant-processor') Processor = cls; };
  await import(pathToFileURL(join(jsRoot, 'src', 'formant-worklet.js')).href);
  if (!Processor) throw new Error('formant-worklet.js did not register formant-processor');

  // Returns, per output, the concatenated samples of channel 0 (null for an
  // output with no channels), after checking every channel equals channel 0.
  return function run(sc) {
    globalThis.sampleRate = sc.sr;
    globalThis.currentTime = 0;
    const node = new Processor({ processorOptions: sc.processorOptions });
    let frames = 0;
    const recorded = sc.outputs.map(() => []);
    for (const op of sc.ops) {
      if (op.op === 'message') {
        globalThis.currentTime = frames / sc.sr;
        node.port.onmessage({ data: op.data });
        continue;
      }
      for (let q = 0; q < op.quanta; q++) {
        // The browser hands process() zero-filled buffers each quantum.
        const outputs = sc.outputs.map(ch => Array.from({ length: ch }, () => new Float32Array(QUANTUM)));
        if (node.process([], outputs) !== true) throw new Error('process() must keep the node alive');
        outputs.forEach((chans, v) => {
          if (!chans.length) return;
          for (let c = 1; c < chans.length; c++) {
            if (chans[c].some((x, i) => !Object.is(x, chans[0][i]))) throw new Error(`output ${v} channel ${c} differs`);
          }
          recorded[v].push(chans[0]);
        });
        frames += QUANTUM;
      }
    }
    return recorded.map((blocks, v) => {
      if (!sc.outputs[v]) return null;
      const all = new Float32Array(blocks.length * QUANTUM);
      blocks.forEach((b, i) => all.set(b, i * QUANTUM));
      return all;
    });
  };
}
