import type { ActionEnvelope, SessionAddedParams } from '@microsoft/agent-host-protocol';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { ROOT, createSession } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/unsubscribe/stops-session-actions',
    area: 'Root & sessions',
    feature: 'unsubscribe',
    title: 'Unsubscribing from a session chat stops subsequent action delivery.',
    spec: '/specification/subscriptions#unsubscribe-notification',
    requires: [requires.model],
    async run(t) {
      const writer = await t.client();
      const session = await t.session(writer);
      const observer = await t.client();
      await observer.subscribe(session.chatUri);
      await observer.unsubscribe(session.chatUri);
      await observer.request('ping', { channel: ROOT });
      const turnId = session.startTurn(prompts.reply(t.uniqueId('reply')));
      await session.waitForTurnEnd(turnId);
      await observer.expectNoNotification<ActionEnvelope>('action', (event) =>
        event.channel === session.chatUri && event.action.type === 'chat/turnStarted' && event.action.turnId === turnId,
      400);
    },
  },
  {
    id: 'sessions/unsubscribe/stops-root-notifications',
    area: 'Root & sessions',
    feature: 'unsubscribe',
    title: 'Unsubscribing from root stops subsequent session-added notifications.',
    spec: '/specification/subscriptions#unsubscribe-notification',
    async run(t) {
      const observer = await t.client();
      const writer = await t.client();
      await observer.unsubscribe(ROOT);
      await observer.request('ping', { channel: ROOT });
      const session = await createSession(t, writer);
      await observer.expectNoNotification<SessionAddedParams>('root/sessionAdded', (event) => event.summary.resource === session.uri, 400);
    },
  },
  {
    id: 'sessions/unsubscribe/resubscribe-restores-delivery',
    area: 'Root & sessions',
    feature: 'unsubscribe',
    title: 'Subscribing again after unsubscribe restores chat-action delivery.',
    spec: '/specification/subscriptions#subscribe-request',
    requires: [requires.model],
    async run(t) {
      const writer = await t.client();
      const session = await t.session(writer);
      const observer = await t.client();
      await observer.subscribe(session.chatUri);
      await observer.unsubscribe(session.chatUri);
      await observer.request('ping', { channel: ROOT });
      await observer.subscribe(session.chatUri);
      const turnId = session.startTurn(prompts.reply(t.uniqueId('reply')));
      await observer.waitForAction(session.chatUri, (event) =>
        event.action.type === 'chat/turnStarted' && event.action.turnId === turnId,
      );
      await session.waitForTurnEnd(turnId);
    },
  },
];
