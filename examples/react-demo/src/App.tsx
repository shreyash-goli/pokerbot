import { type DepthBB, MakoTable, type ThemeProp } from 'mako-browser';
import { useState } from 'react';

// Copied from artifacts/ by scripts/copy-policy.mjs (never committed).
const POLICY_URL = `${import.meta.env.BASE_URL}policy/smoke_20bb.mako`;

function initialSeed(): bigint {
  // ?seed=123 makes a session reproducible (and is used by the smoke test).
  const q = new URLSearchParams(location.search).get('seed');
  if (q && /^\d{1,20}$/.test(q)) return BigInt(q);
  return BigInt(Math.floor(Math.random() * 2 ** 32));
}

export function App() {
  const [seed, setSeed] = useState(initialSeed);
  const [theme, setTheme] = useState<ThemeProp>('sepia');
  const depthBB: DepthBB = 20; // the bundled smoke policy is 20BB only
  const dark = theme === 'terminal';
  return (
    <main
      style={{
        minHeight: '100vh',
        margin: 0,
        padding: '12px 8px 24px',
        boxSizing: 'border-box',
        background: dark ? '#140f0a' : '#e4d3ac',
        color: dark ? '#f2e6c9' : '#3b2b1c',
        fontFamily: "ui-monospace, Menlo, Consolas, monospace",
      }}
    >
      <MakoTable policyUrl={POLICY_URL} depthBB={depthBB} seed={seed} theme={theme} />
      <div style={{ maxWidth: 760, margin: '10px auto 0', display: 'flex', flexWrap: 'wrap', gap: 12, fontSize: 13 }}>
        <span>Seed {seed.toString()}</span>
        <button type="button" onClick={() => setSeed(BigInt(Math.floor(Math.random() * 2 ** 32)))}>
          New session
        </button>
        <button type="button" onClick={() => setTheme(dark ? 'sepia' : 'terminal')}>
          {dark ? 'Sepia theme' : 'Terminal theme'}
        </button>
        <span>Runs entirely in your browser. No server, no account, no real money.</span>
      </div>
    </main>
  );
}
