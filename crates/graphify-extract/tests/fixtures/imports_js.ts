// Various JS/TS import forms.
import defaultExport from './foo';
import * as ns from './bar';
import { named } from './baz';
import { named1, named2 as alias } from 'pkg';
import type { Foo } from './types';
import { a, b, c } from './multi';
import 'side-effect-only';

const dynamic = await import('./dynamic');

export const x = 1;
export { named };

export default function main() {
    return defaultExport(ns, named, alias, a, b, c);
}
