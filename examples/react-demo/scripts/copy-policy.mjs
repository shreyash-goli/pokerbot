// Copies the locally trained 20BB smoke policy into public/policy/ for the
// demo. The .mako file is a generated artifact and is never committed
// (*.mako is gitignored). Train it with scripts/train_smoke.sh.
import { copyFileSync, existsSync, mkdirSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const src = join(here, '../../../artifacts/smoke_20bb.mako');
const dest = join(here, '../public/policy/smoke_20bb.mako');

if (!existsSync(src)) {
  console.warn(
    `WARNING: ${src} not found. The demo will show a "could not start the CFR bot" screen ` +
      `(the untrained baseline is offered explicitly, never substituted). ` +
      `Run scripts/train_smoke.sh from the repository root to train the smoke policy.`,
  );
  process.exit(0);
}
mkdirSync(dirname(dest), { recursive: true });
copyFileSync(src, dest);
console.log(`copied ${src} -> ${dest} (${statSync(dest).size} bytes)`);
