import test from 'node:test';
import assert from 'node:assert/strict';
import { helperBinary } from './helper-binary.mjs';

const root = '/checkout';
const lookup = (env, present = []) => helperBinary(root, 'dojo-speech', { env, exists: path => present.includes(path) });

test('explicit override wins over every installed copy', () => {
  const env = { DOJO_SPEECH_EXE: '/pinned/dojo-speech', XDG_DATA_HOME: '/data' };
  assert.equal(lookup(env, ['/data/dojo/bin/dojo-speech']), '/pinned/dojo-speech');
});

test('override variable is derived from the helper name', () => {
  const env = { DOJO_COMPUTER_RECORDING_EXE: '/pinned/recording' };
  assert.equal(helperBinary(root, 'dojo-computer-recording', { env, exists: () => false }), '/pinned/recording');
});

test('installed copy is preferred over checkout builds', () => {
  const present = ['/data/dojo/bin/dojo-speech', '/checkout/target/release/dojo-speech', '/checkout/target/debug/dojo-speech'];
  assert.equal(lookup({ XDG_DATA_HOME: '/data' }, present), '/data/dojo/bin/dojo-speech');
});

test('release build is preferred over debug build', () => {
  const present = ['/checkout/target/release/dojo-speech', '/checkout/target/debug/dojo-speech'];
  assert.equal(lookup({ XDG_DATA_HOME: '/data' }, present), '/checkout/target/release/dojo-speech');
});

test('home directory supplies the default data root', () => {
  assert.equal(lookup({ HOME: '/home/user' }, ['/home/user/.local/share/dojo/bin/dojo-speech']),
    '/home/user/.local/share/dojo/bin/dojo-speech');
});

test('missing helpers fall back to the debug path for a clear spawn error', () => {
  assert.equal(lookup({ XDG_DATA_HOME: '/data' }), '/checkout/target/debug/dojo-speech');
});
