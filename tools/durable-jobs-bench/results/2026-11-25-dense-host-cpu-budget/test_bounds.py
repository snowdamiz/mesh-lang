import random
import unittest

from replay import BUSY, CPUS, frame_bounds, work_floor


class BudgetTests(unittest.TestCase):
    def test_bounds_cover_actual_reads_but_exclude_early_exit(self):
        epochs = [0, 203_000, 409_000, 410_000]
        points = [dict(epoch_us=t, monotonic_ns=t * 1000, cpu=dict.fromkeys(CPUS))
                  for t in epochs]
        frames = frame_bounds(points)
        self.assertEqual(len(frames), 2)
        for (early, late, _), read in zip(frames, [201_000, 405_000]):
            self.assertLessEqual(early, read)
            self.assertGreaterEqual(late, read)
        points[1]["epoch_us"] += 5000
        with self.assertRaises(AssertionError):
            frame_bounds(points)

    def test_lower_bound_across_straddling_ticks(self):
        rng = random.Random(119)
        # One ms steps, with each CPU idle or charging one named busy field.
        cumulative = [{cpu: [0] * 8 for cpu in CPUS}]
        for _ in range(3000):
            current = {cpu: counters.copy() for cpu, counters in cumulative[-1].items()}
            for cpu in CPUS:
                current[cpu][rng.choice((*BUSY, 3))] += 1000
            cumulative.append(current)
        frames = []
        for t in range(100, 3000, 205):
            ticks = {cpu: [value // 10_000 for value in fields]
                     for cpu, fields in cumulative[t].items()}
            frames.append((t * 1000 - 3000, t * 1000 + 4000, ticks))
        for _ in range(100):
            start = rng.randint(350, 1400)
            end = rng.randint(1700, 2600)
            actual = sum(cumulative[end][cpu][k] - cumulative[start][cpu][k]
                         for cpu in CPUS for k in BUSY)
            floor = work_floor(frames, start * 1000, end * 1000, 10_000)
            self.assertGreater(floor, 0)
            self.assertLessEqual(floor, actual)


if __name__ == "__main__":
    unittest.main()
