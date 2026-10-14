import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('gamepad', Path(__file__).with_name('dojo-gamepad.py'))
gamepad = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gamepad)


class ControlsTest(unittest.TestCase):
    def setUp(self):
        self.events = []
        self.controls = gamepad.Controls(lambda target, value: self.events.append((target, value)))

    def test_app_commands_follow_environment_including_arguments(self):
        with patch.dict(gamepad.os.environ, {'BROWSER': 'my-browser --private', 'TERMINAL': 'my-terminal --new-window', 'EDITOR': 'my-editor "two words"'}), patch.object(gamepad.subprocess, 'run') as run:
            gamepad.open_app('browser')
            run.assert_called_with(['uwsm', 'app', '-t', 'service', '--', 'my-browser', '--private'], check=True)
            gamepad.open_app('terminal')
            run.assert_called_with(['uwsm', 'app', '-t', 'service', '--', 'my-terminal', '--new-window'], check=True)
            gamepad.open_app('editor')
            run.assert_called_with(['uwsm', 'app', '-t', 'service', '--', 'my-terminal', '--new-window', '-e', 'my-editor', 'two words'], check=True)

    def test_missing_app_environment_does_not_choose_another_app(self):
        with patch.dict(gamepad.os.environ, {}, clear=True), patch.object(gamepad.subprocess, 'run') as run:
            with self.assertRaisesRegex(RuntimeError, 'BROWSER'):
                gamepad.open_app('browser')
            run.assert_not_called()

    def test_face_buttons_and_ignore_startup_or_release(self):
        self.controls.button(304, True, initial=True)
        self.controls.button(304, True)
        self.controls.button(304, False)
        self.controls.button(308, True)
        self.controls.button(305, True)
        self.controls.button(307, True)
        self.assertEqual(self.events, [
            ('dojo', 'launcher-activate'), ('dojo', 'launcher'),
            ('dojo', 'launcher-dismiss'), ('niri', 'cycle-focus'),
        ])

    def test_left_stick_focuses_in_all_directions_and_right_stick_is_unassigned(self):
        self.controls.axis(1, -32767)
        self.controls.axis(0, -32767)
        for now in (0, .2, .41):
            self.controls.tick(now)
        self.controls.axis(1, 0)
        self.controls.axis(0, 0)
        self.controls.tick(.5)
        self.controls.axis(1, 32767)
        self.controls.axis(0, 32767)
        self.controls.tick(.6)
        self.controls.axis(3, -32767)
        self.controls.axis(4, -32767)
        self.controls.tick(1)
        self.assertEqual(self.events, [
            ('niri', 'focus-column-left'), ('niri', 'focus-window-up'),
            ('niri', 'focus-column-left'), ('niri', 'focus-window-up'),
            ('niri', 'focus-column-right'), ('niri', 'focus-window-down'),
            ('niri', 'focus-column-right'), ('niri', 'focus-window-down'),
            ('dojo', 'launcher-up'),
        ])

    def test_right_stick_vertical_scrolls_launcher_with_repeat(self):
        self.controls.axis(4, 32767)
        for now in (0, .2, .41, .54):
            self.controls.tick(now)
        self.controls.axis(4, 0)
        self.controls.tick(1)
        self.assertEqual(self.events, [('dojo', 'launcher-down')] * 3)

    def test_dpad_app_shortcuts_once_per_press(self):
        for code, value in [(16, -32767), (17, -32767), (16, 32767), (17, 32767)]:
            self.controls.axis(code, value, initial=True)
            self.controls.axis(code, value)
            self.controls.axis(code, value)
            self.controls.axis(code, 0)
            self.controls.tick(0)
            self.controls.tick(1)
        self.assertEqual(self.events, [('app', 'browser'), ('app', 'editor'), ('app', 'terminal'), ('app', 't3')])

    def test_retired_controls_are_unassigned(self):
        for button in (310, 311, 312, 313, 314, 315, 316, 317, 318):
            self.controls.button(button, True)
        self.assertEqual(self.events, [])

    def test_triangle_cycles_windows_or_monitors_only_when_needed(self):
        with patch.object(gamepad.subprocess, 'check_output', return_value='[{}]') as query, \
             patch.object(gamepad.subprocess, 'run') as run:
            gamepad.cycle_focus()
        query.assert_called_once_with(['niri', 'msg', '--json', 'windows'], text=True)
        run.assert_called_once_with(['niri', 'msg', 'action', 'focus-window-previous'], check=True)

        with patch.object(gamepad.subprocess, 'check_output', side_effect=('[]', '{"A": {}, "B": {}}')), \
             patch.object(gamepad.subprocess, 'run') as run:
            gamepad.cycle_focus()
        run.assert_called_once_with(['niri', 'msg', 'action', 'focus-monitor-next'], check=True)

        with patch.object(gamepad.subprocess, 'check_output', side_effect=('[]', '{"A": {}}')), \
             patch.object(gamepad.subprocess, 'run') as run:
            gamepad.cycle_focus()
        run.assert_not_called()


if __name__ == '__main__':
    unittest.main()
