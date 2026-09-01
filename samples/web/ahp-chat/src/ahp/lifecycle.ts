/**
 * A monotonically increasing token prevents obsolete async work from publishing
 * state after the user connects to a different host.
 */
export class ConnectionEpoch {
  private value = 0

  begin(): number {
    this.value += 1
    return this.value
  }

  invalidate(): void {
    this.value += 1
  }

  isCurrent(token: number): boolean {
    return token === this.value
  }
}
