import { AhpErrorCodes, MessageKind } from '@microsoft/agent-host-protocol';
import type { ChatState, CreateSessionParams, SessionState } from '@microsoft/agent-host-protocol';
import { RpcError } from './types.ts';
import { prompts } from './prompts.ts';
import type { Client, SessionHandle, TestContext, WaitOptions } from './types.ts';

export class ProtocolSession implements SessionHandle {
  readonly sessionUri: string;
  readonly chatUri: string;
  private context: TestContext;
  private client: Client;
  private disposeSession: () => Promise<void>;

  private constructor(context: TestContext, client: Client, sessionUri: string, chatUri: string, dispose: () => Promise<void>) {
    this.context = context;
    this.client = client;
    this.sessionUri = sessionUri;
    this.chatUri = chatUri;
    this.disposeSession = dispose;
  }

  static async create(context: TestContext, client: Client, overrides: Partial<CreateSessionParams> = {}): Promise<ProtocolSession> {
    const params: CreateSessionParams = {
      channel: `ahp-session:/${context.uniqueId()}`,
      provider: context.profile.agent.provider,
      workingDirectories: [context.scratchUri()],
      ...overrides,
    };
    await client.request('createSession', params);
    let disposed = false;
    const dispose = async () => {
      if (disposed) return;
      try { await client.request('disposeSession', { channel: params.channel }, { timeoutMs: 2000 }); }
      catch (error) { if (!(error instanceof RpcError && error.code === AhpErrorCodes.SessionNotFound)) throw error; }
      disposed = true;
    };
    context.defer(dispose);
    await client.subscribe(params.channel);
    const lazy = context.config.quirks.lazySessionInit;
    const state = await client.waitForState<SessionState>(params.channel, (value) => value.lifecycle === 'failed'
      || ((value.lifecycle === 'ready' || (lazy && value.lifecycle === 'creating')) && Boolean(value.defaultChat ?? value.chats[0]?.resource)));
    if (state.lifecycle === 'failed') throw new Error(`Session creation failed: ${JSON.stringify(state.creationError)}`);
    const chatUri = state.defaultChat ?? state.chats[0]?.resource;
    if (!chatUri) throw new Error('Ready session did not advertise a default chat');
    await client.subscribe(chatUri);
    const session = new ProtocolSession(context, client, params.channel, chatUri, dispose);
    return session;
  }

  startTurn(text: string, options?: { chatUri?: string }): string {
    const turnId = this.context.uniqueId('turn');
    this.client.dispatch(options?.chatUri ?? this.chatUri, {
      type: 'chat/turnStarted', turnId, startedAt: new Date().toISOString(),
      message: { text, origin: { kind: MessageKind.User }, ...(this.context.profile.model && { model: { id: this.context.profile.model.id } }) },
    });
    return turnId;
  }

  async waitForTurnEnd<T = unknown>(turnId: string, options?: WaitOptions & { chatUri?: string }): Promise<T> {
    const state = await this.client.waitForState<ChatState>(options?.chatUri ?? this.chatUri,
      (chat) => chat.activeTurn?.id !== turnId && chat.turns.some((turn) => turn.id === turnId), options);
    return state.turns.find((turn) => turn.id === turnId) as T;
  }

  responseText(turnId: string, options?: { chatUri?: string }): string {
    const state = this.client.state<ChatState>(options?.chatUri ?? this.chatUri);
    const turn = state?.activeTurn?.id === turnId ? state.activeTurn : state?.turns.find((value) => value.id === turnId);
    return turn?.responseParts.filter((part) => part.kind === 'markdown').map((part) => part.content).join('') ?? '';
  }

  async ready(): Promise<void> {
    const current = this.client.state<SessionState>(this.sessionUri);
    if (current?.lifecycle === 'creating' && this.context.config.quirks.lazySessionInit) {
      if (!this.context.config.model.enabled) this.context.skip('quirks.lazySessionInit needs a model turn to initialize the session');
      await this.waitForTurnEnd(this.startTurn(prompts.reply('ready')));
    }
    const state = await this.client.waitForState<SessionState>(this.sessionUri, (value) => value.lifecycle !== 'creating');
    if (state.lifecycle === 'failed') throw new Error(`Session creation failed: ${JSON.stringify(state.creationError)}`);
  }

  async dispose(): Promise<void> {
    await this.disposeSession();
  }
}
