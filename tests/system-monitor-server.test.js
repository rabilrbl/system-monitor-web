const test = require('node:test');
const assert = require('node:assert/strict');

const { deriveGpuActivityMetric } = require('../system-monitor-server.js');

test('deriveGpuActivityMetric uses RC6 residency as the primary Intel GPU activity signal', () => {
  const result = deriveGpuActivityMetric({
    elapsedMs: 1000,
    rc6DeltaMs: 250,
    minFreq: 400,
    maxFreq: 1300,
    actFreq: 400,
  });

  assert.equal(result.utilization, 75);
  assert.equal(result.source, 'rc6-residency');
});

test('deriveGpuActivityMetric clamps impossible RC6 deltas instead of producing negative activity', () => {
  const result = deriveGpuActivityMetric({
    elapsedMs: 1000,
    rc6DeltaMs: 5000,
    minFreq: 400,
    maxFreq: 1300,
    actFreq: 400,
  });

  assert.equal(result.utilization, 0);
  assert.equal(result.source, 'rc6-residency');
});

test('deriveGpuActivityMetric falls back to frequency only when RC6 data is unavailable', () => {
  const result = deriveGpuActivityMetric({
    elapsedMs: 0,
    rc6DeltaMs: null,
    minFreq: 400,
    maxFreq: 1300,
    actFreq: 850,
  });

  assert.equal(result.utilization, 50);
  assert.equal(result.source, 'frequency-fallback');
});
