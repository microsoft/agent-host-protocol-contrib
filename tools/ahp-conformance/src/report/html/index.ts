import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import type { ServerResponse } from 'node:http';
import { dirname, resolve } from 'node:path';
import type { Reporter, RunEvent } from '../../harness/types.ts';

export async function createHtmlReporter(options: {
  open: boolean;
  htmlFile?: string;
  log: (line: string) => void;
}): Promise<Reporter> {
  const asset = (file: string) => readFile(resolve(import.meta.dirname, file), 'utf8');
  const [template, css, js] = await Promise.all([asset('page.html'), asset('page.css'), asset('page.js')]);
  const events: string[] = [];
  const clients = new Set<ServerResponse>();
  let closedAt: string | undefined;
  let closePromise: Promise<void> | undefined;

  function page(staticReport: boolean, at?: string): string {
    const data = `{"events":[${staticReport ? events.join(',') : ''}],"closedAt":${JSON.stringify(at ?? null)}}`
      .replaceAll('<', '\\u003c');
    return template
      .replace('<!--REPORT_STYLE-->', () => `<style>\n${css}\n</style>`)
      .replace('<!--REPORT_DATA-->', () => `<script id="run-data" type="application/json">${data}</script>`)
      .replace('<!--REPORT_SCRIPT-->', () => `<script>\n${js}\n</script>`);
  }

  function endEvent(at: string): string {
    return `event: reporter-closed\ndata: ${JSON.stringify({ at })}\n\n`;
  }

  const server = createServer((request, response) => {
    const path = request.url?.split('?')[0];
    if (request.method !== 'GET' || (path !== '/' && path !== '/events')) {
      response.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' });
      response.end('Not found');
      return;
    }
    if (path === '/') {
      response.writeHead(200, {
        'Content-Type': 'text/html; charset=utf-8',
        'Cache-Control': 'no-store',
        'X-Content-Type-Options': 'nosniff',
      });
      response.end(page(false));
      return;
    }
    response.writeHead(200, {
      'Content-Type': 'text/event-stream; charset=utf-8',
      'Cache-Control': 'no-cache, no-transform',
      Connection: 'keep-alive',
      'X-Accel-Buffering': 'no',
      'X-Content-Type-Options': 'nosniff',
    });
    response.flushHeaders();
    const lastId = Number(request.headers['last-event-id'] ?? 0);
    const start = Number.isSafeInteger(lastId) && lastId >= 0 && lastId <= events.length ? lastId : 0;
    for (let index = start; index < events.length; index++) {
      response.write(`id: ${index + 1}\ndata: ${events[index]}\n\n`);
    }
    if (closedAt) {
      response.end(endEvent(closedAt));
      return;
    }
    clients.add(response);
    response.on('close', () => clients.delete(response));
  });

  await new Promise<void>((resolveListening, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      server.off('error', reject);
      resolveListening();
    });
  });
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('HTML reporter has no listening address');
  const url = `http://127.0.0.1:${address.port}/`;
  options.log(`HTML report: ${url}`);

  if (options.open) {
    try {
      const command = process.platform === 'darwin' ? 'open' : process.platform === 'win32' ? 'cmd' : 'xdg-open';
      const args = process.platform === 'win32' ? ['/c', 'start', '""', url] : [url];
      const child = spawn(command, args, {
        detached: true,
        stdio: 'ignore',
        windowsVerbatimArguments: process.platform === 'win32',
      });
      child.on('error', () => {});
      child.unref();
    } catch {
      // Opening a browser is best-effort; the printed URL still works.
    }
  }

  const heartbeat = setInterval(() => {
    for (const client of clients) client.write(': keep-alive\n\n');
  }, 15_000);
  heartbeat.unref();

  return {
    onEvent(event: RunEvent): void {
      const data = JSON.stringify(event);
      events.push(data);
      for (const client of clients) client.write(`id: ${events.length}\ndata: ${data}\n\n`);
    },
    close(): Promise<void> {
      closePromise ??= (async () => {
        const at = new Date().toISOString();
        try {
          if (options.htmlFile) {
            const file = resolve(options.htmlFile);
            await mkdir(dirname(file), { recursive: true });
            await writeFile(file, page(true, at), 'utf8');
            options.log(`HTML report saved: ${file}`);
          }
        } finally {
          closedAt = at;
          clearInterval(heartbeat);
          for (const client of clients) client.end(endEvent(at));
          clients.clear();
          await new Promise<void>((resolveClosed, reject) => {
            server.close((error) => error ? reject(error) : resolveClosed());
            server.closeIdleConnections();
          });
        }
      })();
      return closePromise;
    },
  };
}
