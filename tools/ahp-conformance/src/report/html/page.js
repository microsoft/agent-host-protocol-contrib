(() => {
  'use strict';

  const $ = (id) => document.getElementById(id);
  const statuses = ['passed', 'failed', 'skipped', 'running', 'queued'];
  const labels = { passed: '✓ Yes', failed: '✗ No', skipped: 'N/A', running: 'Running', queued: 'Queued' };
  const selectedStatuses = new Set(statuses);
  const tests = new Map();
  const areas = new Map();
  const dirtyTests = new Set();
  const runningTests = new Set();
  const errors = [];
  const specBase = 'https://microsoft.github.io/agent-host-protocol/';
  let profile;
  let profileDirty = false;
  let summaryDirty = true;
  let viewDirty = true;
  let errorsDirty = false;
  let scheduled = false;
  let startedAt;
  let endedAt;
  let runDuration;
  let finished = false;
  let connectionState = 'Connecting';
  let search = '';
  let scrollTo;
  let nextDetailId = 0;
  let lastEventId = 0;
  let timer;

  function node(tag, className, text) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined) element.textContent = text;
    return element;
  }

  function pretty(value) {
    if (typeof value === 'string') return value;
    return JSON.stringify(value, null, 2) ?? 'undefined';
  }

  function brief(value) {
    return pretty(value).replace(/\s+/g, ' ').slice(0, 160);
  }

  function time(at) {
    const date = new Date(at);
    const element = node('time', '', Number.isNaN(date.getTime()) ? String(at) : date.toISOString().slice(11, 23));
    element.dateTime = at;
    element.title = at;
    return element;
  }

  function duration(ms) {
    return ms < 1000 ? `${Math.round(ms)}ms` : `${(ms / 1000).toFixed(1)}s`;
  }

  function timestamp(at) {
    const value = Date.parse(at);
    return Number.isFinite(value) ? value : undefined;
  }

  function schedule() {
    if (scheduled) return;
    scheduled = true;
    requestAnimationFrame(render);
  }

  function finish(at) {
    finished = true;
    endedAt ??= timestamp(at) ?? Date.now();
    clearInterval(timer);
    schedule();
  }

  function createTest(summary, feature) {
    const test = {
      summary, status: 'queued', logs: [], frames: [], expanded: false,
      resultVersion: 0, renderedVersion: -1, details: undefined, startedAt: undefined,
      durationMs: undefined, reason: undefined, error: undefined,
    };
    const row = node('tr', 'test-row');
    const status = node('td', 'status status-queued', 'Queued');
    const titleCell = node('td');
    const button = node('button', 'test-title', summary.title);
    button.type = 'button';
    button.setAttribute('aria-expanded', 'false');
    const detailId = `test-detail-${++nextDetailId}`;
    button.setAttribute('aria-controls', detailId);
    titleCell.append(button, node('span', 'test-id', summary.id));
    const durationCell = node('td', 'duration', '—');
    const specCell = node('td', 'spec-cell');
    if (summary.spec) {
      try {
        const url = new URL(summary.spec, specBase);
        if (url.origin === new URL(specBase).origin && !url.username && !url.password) {
          const link = node('a', 'spec-link', '↗');
          link.setAttribute('aria-label', `Read specification for ${summary.title} (opens in a new tab)`);
          link.href = url.href;
          link.target = '_blank';
          link.rel = 'noopener noreferrer';
          link.title = summary.spec;
          specCell.append(link);
        }
      } catch {
        // Invalid spec links should not prevent the rest of the report rendering.
      }
    }
    const requirements = node('td', 'requirements');
    for (const label of summary.requires ?? []) requirements.append(node('span', 'requirement', label));
    row.append(status, titleCell, durationCell, specCell, requirements);
    const detailRow = node('tr', 'detail-row');
    detailRow.hidden = true;
    const detailCell = node('td', 'details-cell');
    detailCell.colSpan = 5;
    detailCell.id = detailId;
    detailRow.append(detailCell);
    test.elements = { row, status, button, duration: durationCell, detailRow, detailCell };
    button.addEventListener('click', () => setExpanded(test, !test.expanded));
    row.addEventListener('click', (event) => {
      if (!event.target.closest('a, button')) setExpanded(test, !test.expanded);
    });
    feature.body.append(row, detailRow);
    feature.tests.push(test);
    tests.set(summary.id, test);
    dirtyTests.add(test);
    return test;
  }

  function createPlan(summaries) {
    tests.clear();
    areas.clear();
    dirtyTests.clear();
    runningTests.clear();
    for (const body of $('results').querySelectorAll('tbody')) body.remove();
    for (const summary of summaries) {
      let area = areas.get(summary.area);
      if (!area) {
        const body = node('tbody');
        const headingRow = node('tr', 'area-heading');
        const heading = node('th');
        heading.colSpan = 5;
        heading.scope = 'rowgroup';
        const title = node('div', 'area-title');
        const ratio = node('span', 'ratio');
        ratio.title = 'Passed / executed (passed + failed). Skipped and unfinished tests are excluded.';
        title.append(node('span', '', summary.area), ratio);
        heading.append(title);
        headingRow.append(heading);
        body.append(headingRow);
        area = { body, ratio, features: new Map() };
        areas.set(summary.area, area);
      }
      let feature = area.features.get(summary.feature);
      if (!feature) {
        const body = node('tbody');
        const headingRow = node('tr', 'feature-heading');
        const heading = node('th', '', summary.feature);
        heading.colSpan = 5;
        heading.scope = 'rowgroup';
        headingRow.append(heading);
        body.append(headingRow);
        feature = { body, tests: [] };
        area.features.set(summary.feature, feature);
      }
      createTest(summary, feature);
    }
    const fragment = document.createDocumentFragment();
    for (const area of areas.values()) {
      fragment.append(area.body);
      for (const feature of area.features.values()) fragment.append(feature.body);
    }
    $('results').append(fragment);
    summaryDirty = viewDirty = true;
  }

  function receive(event) {
    const test = tests.get(event.id);
    switch (event.type) {
      case 'run-started':
        startedAt = timestamp(event.at);
        createPlan(event.tests);
        break;
      case 'profile':
        profile = event.profile;
        profileDirty = true;
        break;
      case 'test-started':
        if (!test) break;
        test.status = 'running';
        runningTests.add(test);
        test.startedAt = timestamp(event.at);
        test.resultVersion++;
        dirtyTests.add(test);
        summaryDirty = viewDirty = true;
        if ($('auto-scroll').checked) scrollTo = test;
        break;
      case 'test-log':
        if (!test) break;
        test.logs.push(event);
        dirtyTests.add(test);
        break;
      case 'test-frame':
        if (!test) break;
        test.frames.push(event);
        dirtyTests.add(test);
        break;
      case 'test-finished':
        if (!test) break;
        test.status = event.status;
        runningTests.delete(test);
        test.durationMs = event.durationMs;
        test.reason = event.reason;
        test.error = event.error;
        test.resultVersion++;
        dirtyTests.add(test);
        summaryDirty = viewDirty = true;
        break;
      case 'run-error':
        errors.push(event.message);
        errorsDirty = true;
        break;
      case 'run-finished':
        runDuration = event.durationMs;
        finish(event.at);
        break;
    }
    schedule();
  }

  function renderProfile() {
    const fragment = document.createDocumentFragment();
    const fields = [
      ['Endpoint', profile.endpoint, true],
      ['Protocol', profile.protocolVersion],
      ['Server', profile.serverInfo
        ? [profile.serverInfo.title, profile.serverInfo.name, profile.serverInfo.version].filter(Boolean).join(' · ')
        : 'Not advertised'],
      ['Agent', `${profile.agent.displayName} (${profile.agent.provider})`],
      ['Model', profile.model ?? 'Not advertised'],
      ['Scratch', profile.scratchDir, true],
    ];
    for (const [label, value, wide] of fields) {
      const item = node('div', wide ? 'wide' : '');
      item.append(node('dt', '', label), node('dd', '', value));
      fragment.append(item);
    }
    $('profile').replaceChildren(fragment);
    const chips = document.createDocumentFragment();
    chips.append(node('span', 'capability-label', 'Capabilities'));
    let count = 0;
    function capabilities(value, path, depth) {
      if (value === false || value === null || value === undefined) return;
      if (typeof value === 'object' && !Array.isArray(value) && depth < 6) {
        for (const [key, child] of Object.entries(value)) capabilities(child, path ? `${path}.${key}` : key, depth + 1);
      } else {
        const label = value === true ? path : `${path}: ${brief(value)}`;
        const chip = node('span', 'capability', label || 'Advertised');
        chip.title = pretty(value);
        chips.append(chip);
        count++;
      }
    }
    capabilities(profile.agent.capabilities, '', 0);
    if (!count) chips.append(node('span', 'capability-label', 'None advertised'));
    $('capabilities').replaceChildren(chips);
    profileDirty = false;
  }

  function disclosure(label, value, json = true) {
    const details = node('details');
    details.append(node('summary', '', label));
    let rendered = false;
    details.addEventListener('toggle', () => {
      if (!details.open || rendered) return;
      const text = json ? JSON.stringify(value, null, 2) ?? 'undefined' : String(value);
      details.append(node('pre', '', text));
      rendered = true;
    });
    return details;
  }

  function setExpanded(test, expanded) {
    test.expanded = expanded;
    test.elements.button.setAttribute('aria-expanded', String(expanded));
    test.elements.row.classList.toggle('expanded', expanded);
    dirtyTests.add(test);
    viewDirty = true;
    schedule();
  }

  function createDetails(test) {
    const root = node('div', 'test-details');
    const result = node('section');
    result.setAttribute('aria-label', 'Test result');
    const logHeading = node('h3');
    const logEmpty = node('p', 'empty', 'No log lines recorded.');
    const logList = node('ul', 'log-list');
    const wireHeading = node('h3');
    const wireEmpty = node('p', 'empty', 'No wire frames recorded.');
    const wire = node('div');
    root.append(result, logHeading, logEmpty, logList, wireHeading, wireEmpty, wire);
    test.elements.detailCell.append(root);
    test.details = {
      result, resultVersion: -1, logHeading, logEmpty, logList, logCount: 0,
      wireHeading, wireEmpty, wire, frameCount: 0, connections: new Map(),
    };
    return test.details;
  }

  function renderResult(test, details) {
    const fragment = document.createDocumentFragment();
    fragment.append(node('h3', '', 'Result'), node('p', '', `Status: ${test.status}`));
    if (test.reason) fragment.append(node('p', test.status === 'failed' ? 'reason failure' : 'reason', test.reason));
    if (test.error) {
      if (test.error.message !== test.reason) fragment.append(node('p', 'reason failure', test.error.message));
      const comparison = node('div', 'comparison');
      for (const key of ['expected', 'actual']) {
        if (!Object.hasOwn(test.error, key)) continue;
        const column = node('div');
        column.append(node('h4', '', key === 'expected' ? 'Expected' : 'Actual'));
        column.append(node('pre', '', JSON.stringify(test.error[key], null, 2) ?? 'undefined'));
        comparison.append(column);
      }
      fragment.append(comparison);
      if (test.error.stack) fragment.append(disclosure('Stack trace', test.error.stack, false));
    }
    if (test.status === 'skipped' && !test.reason) fragment.append(node('p', 'reason', 'No skip reason supplied.'));
    details.result.replaceChildren(fragment);
    details.resultVersion = test.resultVersion;
  }

  function frameDescription(message) {
    if (!message || typeof message !== 'object') return brief(message);
    const type = typeof message.method === 'string' ? message.method
      : Object.hasOwn(message, 'error') ? 'error response'
      : Object.hasOwn(message, 'result') ? 'response' : 'message';
    return Object.hasOwn(message, 'id') ? `${type} #${brief(message.id)}` : type;
  }

  function renderDetails(test) {
    const details = test.details ?? createDetails(test);
    if (details.resultVersion !== test.resultVersion) renderResult(test, details);
    details.logHeading.textContent = `Logs (${test.logs.length}) · UTC`;
    details.logEmpty.hidden = test.logs.length > 0;
    const logs = document.createDocumentFragment();
    for (; details.logCount < test.logs.length; details.logCount++) {
      const log = test.logs[details.logCount];
      const line = node('li', 'log-line');
      line.append(time(log.at), node('span', '', log.message));
      if (Object.hasOwn(log, 'data')) line.append(disclosure('Data', log.data));
      logs.append(line);
    }
    details.logList.append(logs);
    details.wireHeading.textContent = `Wire transcript (${test.frames.length} frames) · UTC`;
    details.wireEmpty.hidden = test.frames.length > 0;
    const pending = new Map();
    for (; details.frameCount < test.frames.length; details.frameCount++) {
      const event = test.frames[details.frameCount];
      const frame = event.frame;
      let connection = details.connections.get(frame.connection);
      if (!connection) {
        const group = node('details', 'transcript-connection');
        group.open = true;
        const summary = node('summary');
        const list = node('ul', 'frame-list');
        group.append(summary, list);
        details.wire.append(group);
        connection = { summary, list, count: 0 };
        details.connections.set(frame.connection, connection);
      }
      connection.count++;
      const item = node('li');
      const message = disclosure('', frame.message, typeof frame.message !== 'string');
      const heading = message.querySelector('summary');
      const sending = frame.direction === 'send';
      heading.append(time(event.at), node('span', sending ? 'frame-direction' : 'frame-direction frame-recv', sending ? '→ send' : '← recv'));
      heading.append(node('span', 'frame-description', frameDescription(frame.message)));
      item.append(message);
      if (!pending.has(connection)) pending.set(connection, document.createDocumentFragment());
      pending.get(connection).append(item);
    }
    for (const [connection, fragment] of pending) connection.list.append(fragment);
    for (const [label, connection] of details.connections) {
      connection.summary.textContent = `${label} · ${connection.count} frames`;
    }
  }

  function renderTest(test) {
    if (test.renderedVersion !== test.resultVersion) {
      const { status } = test.elements;
      status.className = `status status-${test.status}`;
      status.textContent = labels[test.status];
      status.title = test.reason ?? test.error?.message ?? test.status;
      status.setAttribute('aria-label', `${test.status}${test.reason ? `: ${test.reason}` : ''}`);
      if (test.durationMs !== undefined) test.elements.duration.textContent = duration(test.durationMs);
      test.renderedVersion = test.resultVersion;
    }
    if (test.expanded) renderDetails(test);
  }

  function applyFilters() {
    let visible = 0;
    for (const area of areas.values()) {
      let areaVisible = false;
      for (const feature of area.features.values()) {
        let featureVisible = false;
        for (const test of feature.tests) {
          const match = selectedStatuses.has(test.status)
            && (!search || `${test.summary.id} ${test.summary.title}`.toLowerCase().includes(search));
          test.elements.row.hidden = !match;
          test.elements.detailRow.hidden = !match || !test.expanded;
          if (match) { visible++; featureVisible = true; }
        }
        feature.body.hidden = !featureVisible;
        areaVisible ||= featureVisible;
      }
      area.body.hidden = !areaVisible;
    }
    $('result-summary').textContent = `${visible} of ${tests.size} tests shown · Area ratios show passed / executed; skips excluded.`;
    $('empty').hidden = visible > 0 || tests.size === 0;
    viewDirty = false;
  }

  function renderSummary() {
    const counts = Object.fromEntries(statuses.map((status) => [status, 0]));
    for (const test of tests.values()) counts[test.status]++;
    const countNodes = document.createDocumentFragment();
    for (const status of statuses) countNodes.append(node('span', `count-${status}`, `${counts[status]} ${status}`));
    $('counts').replaceChildren(countNodes);
    const completed = counts.passed + counts.failed + counts.skipped;
    $('progress').max = Math.max(tests.size, 1);
    $('progress').value = completed;
    $('progress-label').textContent = `${completed} / ${tests.size} complete`;
    for (const area of areas.values()) {
      let passed = 0;
      let failed = 0;
      let skipped = 0;
      for (const feature of area.features.values()) {
        for (const test of feature.tests) {
          if (test.status === 'passed') passed++;
          if (test.status === 'failed') failed++;
          if (test.status === 'skipped') skipped++;
        }
      }
      const total = passed + failed;
      const ratioClass = total === 0 ? '' : passed === total ? 'ratio-high' : passed / total >= 0.5 ? 'ratio-medium' : 'ratio-low';
      area.ratio.className = `ratio ${ratioClass}`;
      area.ratio.textContent = `${passed} / ${total} passed${skipped ? ` · ${skipped} skipped` : ''}`;
    }
    summaryDirty = false;
  }

  function renderClock() {
    const now = endedAt ?? Date.now();
    const elapsed = runDuration ?? (startedAt === undefined ? 0 : Math.max(0, now - startedAt));
    $('elapsed').textContent = `${(elapsed / 1000).toFixed(1)}s`;
    for (const test of runningTests) {
      if (test.startedAt !== undefined) test.elements.duration.textContent = duration(Math.max(0, now - test.startedAt));
    }
  }

  function render() {
    scheduled = false;
    if (profileDirty) renderProfile();
    if (errorsDirty) {
      $('errors').replaceChildren(...errors.map((message) => node('p', '', message)));
      $('errors').hidden = errors.length === 0;
      errorsDirty = false;
    }
    for (const test of dirtyTests) renderTest(test);
    dirtyTests.clear();
    if (summaryDirty) renderSummary();
    if (viewDirty) applyFilters();
    renderClock();
    const indicator = $('connection');
    indicator.textContent = finished ? (errors.length ? 'Finished with errors' : 'Finished') : connectionState;
    indicator.className = `connection ${finished ? (errors.length ? 'interrupted' : 'finished') : connectionState === 'Live' ? 'live' : ''}`;
    document.body.classList.toggle('finished', finished);
    if (scrollTo && $('auto-scroll').checked && !scrollTo.elements.row.hidden) {
      const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;
      scrollTo.elements.row.scrollIntoView({ block: 'center', behavior: reducedMotion ? 'instant' : 'smooth' });
    }
    scrollTo = undefined;
  }

  for (const status of statuses) {
    const button = node('button', 'filter', status[0].toUpperCase() + status.slice(1));
    button.type = 'button';
    button.setAttribute('aria-pressed', 'true');
    button.addEventListener('click', () => {
      if (selectedStatuses.has(status)) selectedStatuses.delete(status);
      else selectedStatuses.add(status);
      button.setAttribute('aria-pressed', String(selectedStatuses.has(status)));
      viewDirty = true;
      schedule();
    });
    $('status-filters').append(button);
  }
  $('search').addEventListener('input', () => {
    search = $('search').value.trim().toLowerCase();
    viewDirty = true;
    schedule();
  });
  $('expand-failures').addEventListener('click', () => {
    for (const test of tests.values()) if (test.status === 'failed') setExpanded(test, true);
  });
  $('auto-scroll').addEventListener('change', () => {
    if ($('auto-scroll').checked) {
      scrollTo = [...tests.values()].find((test) => test.status === 'running');
      schedule();
    }
  });

  const embedded = JSON.parse($('run-data').textContent);
  for (const event of embedded.events) receive(event);
  if (embedded.closedAt || location.protocol === 'file:') {
    finish(embedded.closedAt ?? new Date().toISOString());
  } else {
    const source = new EventSource('/events');
    source.onopen = () => { connectionState = 'Live'; schedule(); };
    source.onmessage = (message) => {
      const id = Number(message.lastEventId);
      if (Number.isSafeInteger(id) && id > 0 && id <= lastEventId) return;
      receive(JSON.parse(message.data));
      if (Number.isSafeInteger(id) && id > 0) lastEventId = id;
    };
    source.addEventListener('reporter-closed', (message) => {
      finish(JSON.parse(message.data).at);
      source.close();
    });
    source.onerror = () => {
      if (finished) source.close();
      else { connectionState = 'Reconnecting…'; schedule(); }
    };
  }
  if (!finished) timer = setInterval(schedule, 250);
  schedule();
})();
