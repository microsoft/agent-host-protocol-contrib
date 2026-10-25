import { JsonRpcErrorCodes } from '@microsoft/agent-host-protocol';
import type { ListSessionsResult } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { ROOT, listedSession, rename } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/list-sessions/items-array',
    area: 'Root & sessions',
    feature: 'listSessions',
    title: 'Listing sessions returns an items array.',
    spec: '/reference/root#listsessions',
    async run(t) {
      const client = await t.client();
      const result = await client.request<ListSessionsResult>('listSessions', { channel: ROOT });
      assert.ok(Array.isArray(result.items));
    },
  },
  {
    id: 'sessions/list-sessions/includes-created',
    area: 'Root & sessions',
    feature: 'listSessions',
    title: 'The session catalog includes a newly created session.',
    spec: '/reference/root#listsessions',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      await listedSession(t, client, session.sessionUri);
    },
  },
  {
    id: 'sessions/list-sessions/summary-fields',
    area: 'Root & sessions',
    feature: 'listSessions',
    title: 'A listed session summary contains its required metadata and timestamps.',
    spec: '/reference/session#sessionsummary',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const summary = await listedSession(t, client, session.sessionUri);
      assert.ok(summary, 'created session must be listed');
      assert.equal(summary.provider, t.profile.agent.provider);
      assert.equal(typeof summary.title, 'string');
      assert.ok(Number.isInteger(summary.status));
      for (const timestamp of [summary.createdAt, summary.modifiedAt]) {
        assert.equal(typeof timestamp, 'string');
        assert.ok(Number.isFinite(Date.parse(timestamp)), 'summary timestamps must be parseable ISO 8601 strings');
      }
    },
  },
  {
    id: 'sessions/list-sessions/renamed-title',
    area: 'Root & sessions',
    feature: 'listSessions',
    title: 'The session catalog reflects a client-renamed title.',
    spec: '/reference/session#sessionmetadata',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const title = t.uniqueId('renamed');
      await rename(client, session.sessionUri, title, t);
      const summary = await listedSession(t, client, session.sessionUri);
      assert.ok(summary);
      assert.equal(summary.title, title);
    },
  },
  {
    id: 'sessions/list-sessions/modified-first',
    area: 'Root & sessions',
    feature: 'listSessions',
    title: 'SHOULD: Sessions are listed in descending modification-time order.',
    spec: '/specification/root-channel#paginating-the-catalogue',
    async run(t) {
      const client = await t.client();
      const older = await t.session(client);
      await older.ready();
      const newer = await t.session(client);
      await newer.ready();
      const page = await client.request<ListSessionsResult>('listSessions', { channel: ROOT });
      if (page.items.length < 2) t.skip('The first page has fewer than two entries to compare.');
      for (let index = 1; index < page.items.length; index++) {
        const [previous, current] = [page.items[index - 1]!, page.items[index]!];
        assert.ok(Date.parse(previous.modifiedAt) >= Date.parse(current.modifiedAt),
          `${previous.resource} (${previous.modifiedAt}) is listed before the more recently modified ${current.resource} (${current.modifiedAt})`);
      }
    },
  },
  {
    id: 'sessions/list-sessions/page-limit',
    area: 'Root & sessions',
    feature: 'listSessions',
    title: 'SHOULD: A paginating server respects the requested page limit.',
    spec: '/specification/root-channel#paginating-the-catalogue',
    async run(t) {
      const client = await t.client();
      await t.session(client);
      await t.session(client);
      const page = await client.request<ListSessionsResult>('listSessions', { channel: ROOT, limit: 1 });
      if (page.items.length > 1 && typeof page.nextCursor !== 'string') {
        t.skip('Server returns an unpaginated catalog, which may ignore pagination inputs.');
      }
      assert.ok(page.items.length <= 1);
    },
  },
  {
    id: 'sessions/list-sessions/cursor-continuation',
    area: 'Root & sessions',
    feature: 'listSessions',
    title: 'Following catalog cursors can retrieve both newly created sessions.',
    spec: '/specification/root-channel#paginating-the-catalogue',
    async run(t) {
      const client = await t.client();
      const first = await t.session(client);
      await first.ready();
      const second = await t.session(client);
      await second.ready();
      const found = new Set<string>();
      const seenPages = new Set<string>();
      const deadline = Date.now() + 5_000;
      let cursor: string | undefined;
      let pages = 0;
      do {
        if (++pages > 100 || Date.now() >= deadline) {
          return t.skip('Catalog exceeds the bounded pagination budget of 100 pages or five seconds.');
        }
        const page = await client.request<ListSessionsResult>('listSessions', {
          channel: ROOT,
          limit: 1,
          ...(cursor === undefined ? {} : { cursor }),
        }, { timeoutMs: Math.max(1, deadline - Date.now()) });
        assert.ok(Array.isArray(page.items), 'listSessions must return an items array');
        assert.ok(page.nextCursor === undefined || typeof page.nextCursor === 'string',
          `listSessions nextCursor must be a string when present; received ${JSON.stringify(page.nextCursor)}`);
        const pageKey = JSON.stringify([cursor, page.items.map((summary) => summary.resource)]);
        assert.ok(!seenPages.has(pageKey), 'listSessions repeated the same page for the same cursor instead of making progress');
        seenPages.add(pageKey);
        for (const summary of page.items) found.add(summary.resource);
        cursor = page.nextCursor;
      } while (cursor !== undefined && (!found.has(first.sessionUri) || !found.has(second.sessionUri)));
      assert.ok(found.has(first.sessionUri) && found.has(second.sessionUri));
    },
  },
  {
    id: 'sessions/list-sessions/invalid-cursor',
    area: 'Root & sessions',
    feature: 'listSessions',
    title: 'SHOULD: A paginating server rejects an unrecognized cursor with InvalidParams.',
    spec: '/specification/root-channel#paginating-the-catalogue',
    async run(t) {
      const client = await t.client();
      await t.session(client);
      await t.session(client);
      const page = await client.request<ListSessionsResult>('listSessions', { channel: ROOT, limit: 1 });
      if (typeof page.nextCursor !== 'string') return t.skip('Server did not expose a pagination cursor.');
      await expectRpcError(client.request('listSessions', {
        channel: ROOT,
        limit: 1,
        cursor: t.uniqueId('invalid-cursor'),
      }), JsonRpcErrorCodes.InvalidParams);
    },
  },
];
