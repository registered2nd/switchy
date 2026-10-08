#!/usr/bin/env node
// Promote the current version's Windows installers into installers/ and
// prune stale-version bundles and installers so neither target/.../{nsis,msi}
// nor installers/ accumulates every build. No-ops cleanly on platforms where a bundle dir is absent
// (e.g. macOS/Linux have no nsis/msi dirs).

import { readdirSync, readFileSync, mkdirSync, copyFileSync, rmSync, existsSync } from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const version = JSON.parse(readFileSync(path.join(root, 'package.json'), 'utf8')).version;
const out = path.join(root, 'installers');

// dir relative to target/release/bundle -> file extension suffix that marks
// a distribution installer in that dir.
const bundles = [
  { dir: 'nsis', suffix: '-setup.exe' },
  { dir: 'msi', suffix: '_en-US.msi' },
];

const bundleBase = path.join(root, 'src-tauri', 'target', 'release', 'bundle');
mkdirSync(out, { recursive: true });

for (const { dir, suffix } of bundles) {
  const full = path.join(bundleBase, dir);
  if (!existsSync(full)) continue;

  const files = readdirSync(full).filter((f) => f.endsWith(suffix));
  const tag = `_${version}_`;

  for (const f of files) {
    const src = path.join(full, f);
    if (f.includes(tag)) {
      copyFileSync(src, path.join(out, f));
      console.log(`promoted ${dir}/${f} -> installers/`);
    } else {
      rmSync(src, { force: true });
      console.log(`pruned stale ${dir}/${f}`);
    }
  }

  // Only once the current installer is in place, so a failed build keeps the old one.
  if (!files.some((f) => f.includes(tag))) continue;
  for (const f of readdirSync(out)) {
    if (f.endsWith(suffix) && !f.includes(tag)) {
      rmSync(path.join(out, f), { force: true });
      console.log(`pruned stale installers/${f}`);
    }
  }
}
