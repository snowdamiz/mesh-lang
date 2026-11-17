import exec from 'k6/execution';
import http from 'k6/http';
import { Counter, Trend } from 'k6/metrics';

// run.py supplies bench and the Python oracle in the stdin script.
const cases = bench.cases.map(({record, expected}) => ({
  body: JSON.stringify(record),
  expected,
}));
const started = new Counter('score_started');
const success = new Counter('score_success');
const errors = new Counter('score_errors');
const good = new Counter('score_within_50ms');
const latency = new Trend('score_e2e_ms', true);

export const options = {
  noConnectionReuse: true,
  discardResponseBodies: false,
  maxRedirects: 0,
  summaryTrendStats: ['min', 'med', 'p(95)', 'p(99)', 'max'],
  scenarios: {
    score: {
      executor: 'constant-arrival-rate',
      rate: bench.rate,
      timeUnit: '1s',
      duration: bench.seconds + 's',
      gracefulStop: '3s',
      preAllocatedVUs: bench.vus,
    },
  },
};

export default function () {
  const {body, expected} = cases[exec.scenario.iterationInTest % cases.length];
  started.add(1);
  const response = http.post(bench.url, body, {
    headers: {'Content-Type': 'application/json', 'Connection': 'close'},
    timeout: '2s',
  });
  let valid = false;
  if (response.status === 200 && response.proto === 'HTTP/1.1' &&
      /^application\/json(?:\s*;|$)/i.test(response.headers['Content-Type'] || '')) {
    try {
      const actual = response.json();
      valid = actual !== null && typeof actual === 'object' &&
        Object.keys(actual).length === 2 &&
        actual.id === expected.id && actual.score === expected.score;
    } catch (_) {
      // A malformed body is a failed request.
    }
  }
  if (valid) {
    // k6 excludes the TCP dial from its HTTP duration.
    const elapsed = response.timings.connecting + response.timings.duration;
    latency.add(elapsed);
    success.add(1);
    good.add(elapsed <= 50 ? 1 : 0);
  } else {
    errors.add(1);
  }
}

export function handleSummary(data) {
  return {stdout: JSON.stringify(data) + '\n'};
}
