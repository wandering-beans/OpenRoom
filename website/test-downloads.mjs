import assert from 'node:assert/strict';
import {setupDownloadForm} from './downloads.mjs';

const agreement = Object.assign(new EventTarget(), {checked:true});
const buttons = [{value:'windows'}, {value:'macos'}];
const form = Object.assign(new EventTarget(), {
  querySelector: () => agreement,
  querySelectorAll: () => buttons,
  reportValidity: () => true
});
globalThis.window = new EventTarget();
const navigations = [];
const gate = setupDownloadForm(form, url => navigations.push(url));
const submit = platform => {
  const event = new Event('submit', {cancelable:true});
  Object.defineProperty(event, 'submitter', {value:{value:platform}});
  form.dispatchEvent(event);
  assert.equal(event.defaultPrevented, true);
};
assert.equal(agreement.checked, false, 'Browser-restored consent must be cleared');
assert.ok(buttons.every(button => button.disabled));
gate.setDestination('windows', 'https://github.com/wandering-beans/OpenRoom/releases/download/v1/OpenRoom-1-windows-x64-setup.exe');
submit('windows');
assert.equal(navigations.length, 0, 'Submission without consent must not navigate');
agreement.checked = true;
agreement.dispatchEvent(new Event('change'));
assert.equal(buttons[0].disabled, false);
assert.equal(buttons[1].disabled, true, 'Missing destinations stay disabled');
submit('windows');
assert.equal(navigations.length, 1);
submit('macos');
assert.equal(navigations.length, 1);
gate.setDestination('macos', 'https://github.com/wandering-beans/OpenRoom/releases');
assert.equal(buttons[1].disabled, false);
agreement.checked = false;
agreement.dispatchEvent(new Event('change'));
assert.ok(buttons.every(button => button.disabled));
submit('macos');
assert.equal(navigations.length, 1);
agreement.checked = true;
const restore = new Event('pageshow');
Object.defineProperty(restore, 'persisted', {value:true});
window.dispatchEvent(restore);
assert.equal(agreement.checked, false, 'Back-forward cache must not restore consent');
assert.ok(buttons.every(button => button.disabled));
delete globalThis.window;
console.log('Download consent passed: initial lock, submit guard, opt-in, opt-out, absent destination, fallback, restored-page reset.');
