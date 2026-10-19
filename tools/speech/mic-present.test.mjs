import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const script = new URL('./dojo-mic-present', import.meta.url).pathname;
const directory = await mkdtemp(join(tmpdir(), 'dojo-mic-test-'));
const monitor = `Source #50
\tState: SUSPENDED
\tName: alsa_output.pci-0000_00_1f.3.iec958-stereo.monitor
\tMonitor of Sink: alsa_output.pci-0000_00_1f.3.iec958-stereo
\tPorts:
\t\tiec958-stereo-output: Digital Output (S/PDIF) (type: SPDIF, priority: 0, availability unknown)
\tActive Port: iec958-stereo-output
`;
const jack = unplugged => `Source #51
\tName: alsa_input.pci-0000_00_1f.3.analog-stereo
\tMonitor of Sink: n/a
\tPorts:
\t\tanalog-input-front-mic: Front Microphone (type: Mic, priority: 8500, availability group: Legacy 1, ${unplugged ? 'not available' : 'availability unknown'})
\t\tanalog-input-rear-mic: Rear Microphone (type: Mic, priority: 8200, availability group: Legacy 2, not available)
\tActive Port: analog-input-front-mic
`;
const usb = `Source #60
\tName: alsa_input.usb-Headset-00.mono-fallback
\tMonitor of Sink: n/a
`;

async function present(text) {
  const path = join(directory, `${Math.random()}.txt`);
  await writeFile(path, text);
  const result = spawnSync(script, [path]);
  assert.ok(result.status === 0 || result.status === 1, result.stderr.toString());
  return result.status === 0;
}

test('only an output monitor means no microphone', async () => {
  assert.equal(await present(monitor), false);
  assert.equal(await present(''), false);
});
test('an unplugged jack does not count, a plugged one does', async () => {
  assert.equal(await present(monitor + jack(true)), false);
  assert.equal(await present(monitor + jack(false)), true);
});
test('a source without ports, such as a USB headset, counts', async () => {
  assert.equal(await present(monitor + jack(true) + usb), true);
});
