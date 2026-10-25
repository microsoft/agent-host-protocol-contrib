import { writeFile } from 'node:fs/promises';
import { z } from 'zod';
import { configSchema } from './schema.ts';

await writeFile(new URL('../../conformance.schema.json', import.meta.url), `${JSON.stringify(z.toJSONSchema(configSchema), null, 2)}\n`);
