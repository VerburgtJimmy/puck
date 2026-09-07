import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const repo = join(root, '..');
const dest = join(root, 'public', 'install');
mkdirSync(dirname(dest), { recursive: true });
copyFileSync(join(repo, 'install.sh'), dest);
console.log('synced install.sh → public/install');
