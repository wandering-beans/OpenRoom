// Measure synchronous input work separately from the mocked renderer.
const assert = require('node:assert/strict');
const { writeFile } = require('node:fs/promises');

module.exports = async function measure(page) {
  await page.click('#module-develop');
  await page.waitForFunction(() => !document.querySelector('#develop-controls').disabled);
  await page.waitForFunction(() => document.querySelector('#full-image').src.includes('develop-'));
  const result = await page.evaluate(async () => {
    const input = document.getElementById('develop-exposure');
    const samples = [];
    input.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    for (let index = 0; index < 130; index++) {
      await new Promise(requestAnimationFrame);
      input.value = String((index - 65) * 0.05);
      const started = performance.now();
      input.dispatchEvent(new Event('input', { bubbles: true }));
      const elapsed = performance.now() - started;
      if (index >= 10) samples.push(elapsed);
    }
    input.dispatchEvent(new Event('change', { bubbles: true }));
    samples.sort((a, b) => a - b);
    return {
      surface: 'Chromium, real controls with mocked preview commands',
      samples: samples.length,
      input_handler_median_ms: (samples[59] + samples[60]) / 2,
      input_handler_p95_ms: samples[113],
      range_value: input.value,
      numeric_value: document.getElementById('develop-exposure-value').value,
    };
  });
  assert.equal(result.samples, 120);
  assert.equal(Number(result.range_value), 3.2);
  assert.equal(Number(result.numeric_value), 3.2);
  await page.waitForFunction(() => calls.some(call => call.command === 'save_develop' && call.args.settings.exposure === 3.2));
  console.log(JSON.stringify(result));
  if (process.env.AE_LATENCY_REPORT) await writeFile(process.env.AE_LATENCY_REPORT, JSON.stringify(result, null, 2));
};
