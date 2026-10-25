// Natural-language prompts with embedded deterministic-openai directive blocks.
// A real model follows the plain-language instruction; the fixture parses the
// exact `[AHP-TEST vN]` block (see crates/deterministic-openai/README.md).
// Assert on model output with `includes`/trimmed comparisons so real models pass too.

function block(version: string, lines: (string | false | undefined)[]): string {
  return [`[AHP-TEST ${version}]`, ...lines.filter((line): line is string => typeof line === 'string'), '[/AHP-TEST]'].join('\n');
}

export interface ReplyOptions {
  /** Ask the fixture to stream the reply in exactly this many chunks. */
  chunks?: number;
  /** Ask the fixture to wait this long before every chunk (useful for cancellation). */
  waitMs?: number;
}

export const prompts = {
  /** The model replies with exactly `text`. */
  reply(text: string, options: ReplyOptions = {}): string {
    return [
      `Reply with exactly the following text, without quotes, and nothing else: ${text}`,
      '',
      block('v1', [
        `Respond exactly with JSON: ${JSON.stringify(text)}`,
        options.chunks !== undefined && `Stream in exactly ${options.chunks} chunks.`,
        options.waitMs !== undefined && `Wait ${options.waitMs} ms before every output chunk.`,
      ]),
    ].join('\n');
  },

  /** The model's upstream request fails with an HTTP error. Fixture only: gate with `requires.deterministicModel`. */
  upstreamError(status: number, message: string): string {
    return block('v1', [`Return upstream HTTP ${status} with JSON message: ${JSON.stringify(message)}.`]);
  },

  /** Establishes a phrase for a later `recall()`; the model replies `OK`. */
  remember(phrase: string): string {
    return [
      `Remember this phrase: ${JSON.stringify(phrase)}`,
      'Reply with only the word OK.',
      '',
      block('v1', ['Respond exactly with JSON: "OK"']),
    ].join('\n');
  },

  /** The model replies with the phrase from the most recent `remember()` in its context. */
  recall(): string {
    return block('v4', ['Recall the phrase I asked you to remember and respond with only that phrase.']);
  },

  /** The model calls the tool named `name` once with `args`, then replies with exactly `text`. */
  callTool(name: string, args: Record<string, unknown>, text: string): string {
    return [
      `Call the tool named ${name} exactly once with these JSON arguments: ${JSON.stringify(args)}`,
      `After it returns, reply with exactly the following text, without quotes, and nothing else: ${text}`,
      '',
      block('v2', [`Call tool ${name} with JSON: ${JSON.stringify(args)}`, `Respond exactly after tools with JSON: ${JSON.stringify(text)}.`]),
    ].join('\n');
  },

  /** The model repeats its previous assistant response verbatim. */
  repeatPrevious(): string {
    return block('v3', ['Repeat the previous assistant response exactly.']);
  },

  /** The model reads `absolutePath` with one of its tools and replies with the file's exact contents. */
  readFile(absolutePath: string): string {
    return block('v4', [
      `Read the file at JSON path: ${JSON.stringify(absolutePath)} with an available tool, then respond with only its exact contents.`,
    ]);
  },

  /** The model creates `absolutePath` containing `contents` with one of its tools and replies `done`. */
  writeFile(absolutePath: string, contents: string): string {
    return [
      `Use an available tool to create the file ${absolutePath}. Its content must be the decoded value of the JSON string`,
      `literal ${JSON.stringify(contents)}: each \\n is a newline character, and the surrounding quotes and backslash escapes`,
      'are not part of the content. Then respond with only the word done.',
      '',
      block('v4', [
        `Create the file at JSON path: ${JSON.stringify(absolutePath)} containing JSON text: ${JSON.stringify(contents)} with an available tool, then respond with only the word done.`,
      ]),
    ].join('\n');
  },
};
