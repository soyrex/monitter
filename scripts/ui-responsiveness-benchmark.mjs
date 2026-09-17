import { performance } from 'node:perf_hooks';

const transcript = Array.from({ length: 1_500 }, (_, index) => ({ id: `m-${index}`, text: `row ${index} ${'streamed markdown '.repeat(40)}`, attachments: [{ id: `a-${index}`, previewDataUrl: `data:${index}` }] }));
const fixture = { transcript, approval: { response: { nested: { value: 'allowed' } } } };
function median(values) { return [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)]; }
function measure(work) { const values = []; for (let sample = 0; sample < 9; sample += 1) { const start = performance.now(); for (let run = 0; run < 5; run += 1) work(); values.push((performance.now() - start) / 5); } return median(values); }

const json = JSON.stringify(fixture);
const jsonMs = measure(() => JSON.stringify(fixture));
console.log(`ui responsiveness JSON baseline: fixture=${(json.length / 1024 / 1024).toFixed(2)}MiB median=${jsonMs.toFixed(2)}ms`);
