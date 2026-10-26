import { AhpClient as SdkClient } from '@microsoft/agent-host-protocol/client';
import { WebSocketTransport } from '@microsoft/agent-host-protocol/ws';

interface CompatibleSocket {
  readonly OPEN: number;
  readonly CONNECTING: number;
  readonly readyState: number;
  readonly bufferedAmount: number;
  binaryType: string;
  send(data: string): void;
  close(): void;
  addEventListener(type: 'message', listener: (event: { data: unknown }) => void): void;
  addEventListener(type: 'error', listener: () => void): void;
  addEventListener(type: 'close', listener: (event: { code: number; reason: string; wasClean: boolean }) => void): void;
}

// SDK 1.0.0 already sends arbitrary requests at runtime; only its public type is restricted.
export class AhpClient extends SdkClient {
  requestRaw(method: string, params: unknown): Promise<unknown> {
    const request = this.request as (method: string, params: unknown) => Promise<unknown>;
    return request.call(this, method, params);
  }
}

export function wrapSocket(socket: CompatibleSocket): WebSocketTransport {
  // Match the upstream structural socket API without claiming ws implements the entire DOM WebSocket.
  const fromSocket = WebSocketTransport.fromSocket as (socket: CompatibleSocket) => WebSocketTransport;
  return fromSocket(socket);
}
