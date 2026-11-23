"""Exercise the Linux clock and process-identity seams without a Linux guest."""

from contextlib import ExitStack
import ctypes
import errno
from pathlib import Path
import tarfile
from types import ModuleType, SimpleNamespace
import unittest
from unittest.mock import Mock, patch


HERE = Path(__file__).resolve().parent
payload = HERE.parent / "recovered/dependencies.tar"
capture = ModuleType("repaired_capture")
capture.__file__ = f"{payload}:native.py"
with tarfile.open(payload, "r:") as archive:
    source = archive.extractfile("native.py")
    with source:
        exec(compile(source.read(), capture.__file__, "exec"), vars(capture))


class ClockTests(unittest.TestCase):
    def test_libc_process_clock_passes_signed_id_to_nanosecond_reader(self):
        c_call = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_int, ctypes.POINTER(ctypes.c_int))

        @c_call
        def libc_get_clock(pid, target):
            if pid != 81:
                return errno.ESRCH
            target[0] = -654
            return 0

        with (patch.object(capture.ctypes, "CDLL", return_value=SimpleNamespace(
                  clock_getcpuclockid=libc_get_clock)),
              patch.object(capture.time, "clock_gettime_ns", return_value=123400) as read):
            clock = capture.runner_cpu_clock()
            self.assertEqual(clock(81), 123400)
            read.assert_called_once_with(-654)
            with self.assertRaises(ProcessLookupError) as error:
                clock(82)
            self.assertEqual(error.exception.errno, errno.ESRCH)
            read.assert_called_once()

    def test_live_runner_accepts_its_original_identity(self):
        watch = Mock()
        watch.poll.side_effect = [[], []]
        clock = Mock(return_value=752)
        self.assertEqual(capture.runner_cpu_for_live_pid(81, clock, {81: watch}), 752)
        clock.assert_called_once_with(81)
        self.assertEqual(watch.poll.call_count, 2)

    def test_dead_or_unbound_runner_never_reaches_pid_only_clock(self):
        watch = Mock()
        watch.poll.return_value = [(7, capture.select.POLLIN)]
        clock = Mock()
        with self.assertRaises(ProcessLookupError):
            capture.runner_cpu_for_live_pid(81, clock, {81: watch})
        with self.assertRaises(ProcessLookupError):
            capture.runner_cpu_for_live_pid(81, clock, {})
        clock.assert_not_called()

    def test_exit_during_clock_read_discards_sample_even_on_pid_reuse(self):
        watch = Mock()
        watch.poll.side_effect = [[], [(7, capture.select.POLLIN)]]
        replacement_pid_clock = Mock(return_value=752)
        with self.assertRaises(ProcessLookupError):
            capture.runner_cpu_for_live_pid(81, replacement_pid_clock, {81: watch})
        self.assertEqual(watch.poll.call_count, 2)

    def test_open_failure_stays_unbound_and_owned_pidfd_closes(self):
        role = "engine_runner_root"
        with (patch.object(capture.os, "pidfd_open", create=True,
                           side_effect=[7, ProcessLookupError()]) as open_pidfd,
              patch.object(capture.os, "close") as close_pidfd,
              patch.object(capture.select, "poll") as create_watch):
            with ExitStack() as stack:
                watches = capture.bind_runner_pids({81: role, 82: role, 83: "postgres"}, stack)
                self.assertEqual(list(watches), [81])
                watches[81].register.assert_called_once_with(7, capture.select.POLLIN)
                close_pidfd.assert_not_called()
            self.assertEqual(open_pidfd.call_count, 2)
            create_watch.assert_called_once()
            close_pidfd.assert_called_once_with(7)


if __name__ == "__main__":
    unittest.main(verbosity=2)
