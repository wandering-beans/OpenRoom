import assert from 'node:assert/strict';
import {setupDownloadForm, recordDownloadClick} from './downloads.mjs';

const agreement = Object.assign(new EventTarget(), {checked:true});
const buttons = [{value:'windows'}, {value:'macos'}];
const form = Object.assign(new EventTarget(), {
  querySelector: () => agreement,
  querySelectorAll: () => buttons,
  reportValidity: () => true
});
globalThis.window = new EventTarget();
const navigations = [];
const clicks = [];
let trackingFails = false;
const gate = setupDownloadForm(form, url => navigations.push(url), platform => {
  if (trackingFails) throw new Error('Tracking unavailable');
  clicks.push(platform);
});
const submit = platform => {
  const event = new Event('submit', {cancelable:true});
  Object.defineProperty(event, 'submitter', {value:{value:platform}});
  form.dispatchEvent(event);
  assert.equal(event.defaultPrevented, true);
};
assert.equal(agreement.checked, false, 'Browser-restored consent must be cleared');
assert.ok(buttons.every(button => button.disabled));
gate.setDestination('windows', 'https://github.com/wandering-beans/OpenRoom/releases/download/v1/OpenRoom-1-windows-x64-setup.exe', true);
submit('windows');
assert.equal(navigations.length, 0, 'Submission without consent must not navigate');
assert.deepEqual(clicks, [], 'Submission without consent must not count');
agreement.checked = true;
agreement.dispatchEvent(new Event('change'));
assert.equal(buttons[0].disabled, false);
assert.equal(buttons[1].disabled, true, 'Missing destinations stay disabled');
submit('windows');
assert.equal(navigations.length, 1);
assert.deepEqual(clicks, ['windows']);
submit('macos');
assert.equal(navigations.length, 1);
gate.setDestination('macos', 'https://github.com/wandering-beans/OpenRoom/releases');
assert.equal(buttons[1].disabled, false);
submit('macos');
assert.equal(navigations.length, 2);
assert.deepEqual(clicks, ['windows'], 'The release-list fallback is not a download click');
trackingFails = true;
submit('windows');
assert.equal(navigations.length, 3, 'Tracking failure must not prevent navigation');
agreement.checked = false;
agreement.dispatchEvent(new Event('change'));
assert.ok(buttons.every(button => button.disabled));
submit('macos');
assert.equal(navigations.length, 3);
agreement.checked = true;
const restore = new Event('pageshow');
Object.defineProperty(restore, 'persisted', {value:true});
window.dispatchEvent(restore);
assert.equal(agreement.checked, false, 'Back-forward cache must not restore consent');
assert.ok(buttons.every(button => button.disabled));
delete globalThis.window;

const realFetch = globalThis.fetch;
const requests = [];
globalThis.fetch = (url, options) => {
  requests.push({url, options});
  return Promise.reject(new Error('Offline'));
};
recordDownloadClick('windows');
recordDownloadClick('macos');
recordDownloadClick('invalid');
await new Promise(resolve => setImmediate(resolve));
assert.deepEqual(requests.map(request => request.url), ['/api/download-click/windows', '/api/download-click/macos']);
assert.ok(requests.every(request => request.options.keepalive && request.options.method === 'POST' && request.options.credentials === 'omit'));
globalThis.fetch = realFetch;
console.log('Download consent and counting passed: consent gate, platform tracking, fallback exclusion, keepalive, network failures, restored-page reset.');
